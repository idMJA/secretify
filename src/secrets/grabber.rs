use headless_chrome::protocol::cdp::Page;
use headless_chrome::Browser;
use regex::Regex;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::time::Duration;
use tracing::{debug, error, info, instrument, warn};

const STEALTH_JS: &str = r"(function () {
	Object.defineProperty(navigator, 'webdriver', { get: () => false });
	Object.defineProperty(navigator, 'languages', { get: () => ['en-US', 'en'] });
	Object.defineProperty(navigator, 'plugins', { get: () => [1, 2, 3, 4, 5] });
	window.chrome = { runtime: {} };

	const originalQuery = window.navigator.permissions.query;
	window.navigator.permissions.query = (parameters) => (
		parameters.name === 'notifications' ?
			Promise.resolve({ state: Notification.permission }) :
			originalQuery(parameters)
	);

	const getParameter = WebGLRenderingContext.prototype.getParameter;
	WebGLRenderingContext.prototype.getParameter = function (param) {
		if (param === 37445) return 'Intel Inc.';
		if (param === 37446) return 'Intel Iris OpenGL Engine';
		return getParameter.call(this, param);
	};
})();";

const HOOK_JS: &str = r"(()=>{if(globalThis.__secretHookInstalled)return;globalThis.__secretHookInstalled=true;globalThis.__captures=[];Object.defineProperty(Object.prototype,'secret',{configurable:true,set:function(v){try{__captures.push({secret:v,version:this.version,obj:this});}catch(e){}Object.defineProperty(this,'secret',{value:v,writable:true,configurable:true,enumerable:true});}});})();";

const JS_STRING_PATTERN: &str = r#"(?:'(?:\\.|[^'\\])*'|"(?:\\.|[^"\\])*")"#;

fn decode_js_string_literal(literal: &str) -> Option<String> {
    let trimmed = literal.trim();
    if (trimmed.starts_with('"') && trimmed.ends_with('"'))
        || (trimmed.starts_with('\'') && trimmed.ends_with('\''))
    {
        let inner = &trimmed[1..trimmed.len() - 1];
        let mut res = String::with_capacity(inner.len());
        let mut chars = inner.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('n') => res.push('\n'),
                    Some('r') => res.push('\r'),
                    Some('t') => res.push('\t'),
                    Some('\\') => res.push('\\'),
                    Some('\'') => res.push('\''),
                    Some('"') => res.push('"'),
                    Some('u') => {
                        let hex: String = chars.by_ref().take(4).collect();
                        if let Ok(u) = u32::from_str_radix(&hex, 16) {
                            if let Some(ch) = char::from_u32(u) {
                                res.push(ch);
                            }
                        }
                    }
                    Some(other) => res.push(other),
                    None => break,
                }
            } else {
                res.push(c);
            }
        }
        Some(res)
    } else {
        None
    }
}

pub fn extract_bundle_secrets(source: &str) -> Vec<Value> {
    let secret_first_pattern = format!(
        r#"\{{\s*(?:secret|['"]secret['"])\s*:\s*(?P<secret>{})\s*,\s*(?:version|['"]version['"])\s*:\s*(?P<version>\d+)\s*\}}"#,
        JS_STRING_PATTERN
    );
    let version_first_pattern = format!(
        r#"\{{\s*(?:version|['"]version['"])\s*:\s*(?P<version>\d+)\s*,\s*(?:secret|['"]secret['"])\s*:\s*(?P<secret>{})\s*\}}"#,
        JS_STRING_PATTERN
    );

    let secret_first_re = Regex::new(&secret_first_pattern).unwrap();
    let version_first_re = Regex::new(&version_first_pattern).unwrap();

    let mut captures = Vec::new();
    let mut seen = HashSet::new();

    for pattern in &[secret_first_re, version_first_re] {
        for cap in pattern.captures_iter(source) {
            if let (Some(sec_match), Some(ver_match)) = (cap.name("secret"), cap.name("version")) {
                if let Some(secret) = decode_js_string_literal(sec_match.as_str()) {
                    if let Ok(version) = ver_match.as_str().parse::<i32>() {
                        if seen.insert((version, secret.clone())) {
                            captures.push(json!({
                                "secret": secret,
                                "version": version,
                                "source": "bundle"
                            }));
                        }
                    }
                }
            }
        }
    }

    captures
}

#[instrument(skip_all)]
pub async fn grab_live() -> Result<Vec<Value>, Box<dyn std::error::Error>> {
    info!("Launching headless browser...");

    let launch_options = headless_chrome::LaunchOptionsBuilder::default()
        .headless(true)
        .sandbox(false)
        .args(vec![
            std::ffi::OsStr::new("--no-sandbox"),
            std::ffi::OsStr::new("--disable-setuid-sandbox"),
            std::ffi::OsStr::new("--disable-dev-shm-usage"),
            std::ffi::OsStr::new("--disable-gpu"),
        ])
        .build()?;

    let browser = Browser::new(launch_options)?;
    let tab = browser.new_tab()?;

    info!("Installing stealth script to evaluate on new document...");
    tab.call_method(Page::AddScriptToEvaluateOnNewDocument {
        source: STEALTH_JS.to_string(),
        world_name: None,
        include_command_line_api: None,
        run_immediately: None,
    })?;

    info!("Installing secret hook to evaluate on new document...");
    tab.call_method(Page::AddScriptToEvaluateOnNewDocument {
        source: HOOK_JS.to_string(),
        world_name: None,
        include_command_line_api: None,
        run_immediately: None,
    })?;

    info!("Opening https://open.spotify.com");
    match tab.navigate_to("https://open.spotify.com") {
        Ok(_) => info!("Navigation successful"),
        Err(e) => {
            error!("Navigation error: {:?}", e);
            return Err(e.into());
        }
    }

    info!("Waiting for page scripts to load...");
    
    let bundle_regex =
        Regex::new(r#"(?:vendor~web-player|encore~web-player|web-player)\.[0-9a-f]{4,}\.(?:js|mjs)"#)?;

    let mut all_captures = Vec::new();
    let mut seen_secrets = HashSet::new();

    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36")
        .build()?;

    // Loop polling for bundles up to 15 seconds
    let mut scanned_urls = HashSet::new();
    for i in 1..=6 {
        tokio::time::sleep(Duration::from_secs(2)).await;

        let script_urls_val = tab.evaluate(
            r#"(function() {
                const scripts = Array.from(document.querySelectorAll('script[src]')).map(s => s.src);
                const resources = performance.getEntriesByType('resource')
                    .filter(r => r.initiatorType === 'script' || r.name.endsWith('.js') || r.name.endsWith('.mjs'))
                    .map(r => r.name);
                return JSON.stringify(Array.from(new Set([...scripts, ...resources])));
            })()"#,
            false,
        );

        if let Ok(remote_obj) = script_urls_val {
            if let Some(val_str) = remote_obj.value.as_ref().and_then(|v| v.as_str()) {
                if let Ok(urls) = serde_json::from_str::<Vec<String>>(val_str) {
                    for url_str in urls {
                        let filename = url_str.split('/').last().unwrap_or("").split('?').next().unwrap_or("");
                        if bundle_regex.is_match(filename) && scanned_urls.insert(url_str.clone()) {
                            info!("Found matching Spotify bundle (attempt {}): {}", i, filename);
                            match client.get(&url_str).send().await {
                                Ok(resp) => {
                                    if let Ok(body) = resp.text().await {
                                        let extracted = extract_bundle_secrets(&body);
                                        info!("Extracted {} secrets statically from {}", extracted.len(), filename);
                                        for cap in extracted {
                                            if let (Some(sec), Some(ver)) = (
                                                cap.get("secret").and_then(Value::as_str),
                                                cap.get("version").and_then(Value::as_i64),
                                            ) {
                                                if seen_secrets.insert((ver as i32, sec.to_string())) {
                                                    all_captures.push(cap);
                                                }
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    warn!("Failed to fetch bundle {}: {:?}", url_str, e);
                                }
                            }
                        }
                    }
                }
            }
        }

        if !all_captures.is_empty() {
            break;
        }
    }

    // 2. Runtime property hook extraction (fallback)
    info!("Evaluating runtime hooked secrets...");
    let hook_captures = match tab.evaluate(
        "(function() { try { return JSON.stringify(globalThis.__captures || []); } catch(e) { return '[]'; } })()",
        false,
    ) {
        Ok(remote_obj) => {
            remote_obj.value.map_or_else(
                || Vec::new(),
                |value| {
                    value.as_str().map_or_else(
                        || Vec::new(),
                        |json_str| {
                            serde_json::from_str::<Vec<Value>>(json_str).unwrap_or_default()
                        },
                    )
                },
            )
        }
        Err(e) => {
            debug!("Failed to evaluate runtime hook: {:?}", e);
            Vec::new()
        }
    };

    info!("Runtime hook captured {} items", hook_captures.len());
    for cap in hook_captures {
        if let (Some(sec), Some(ver)) = (
            cap.get("secret").and_then(Value::as_str),
            cap.get("version").and_then(Value::as_i64),
        ) {
            if seen_secrets.insert((ver as i32, sec.to_string())) {
                all_captures.push(cap);
            }
        }
    }

    if all_captures.is_empty() {
        warn!("No secrets captured");
    } else {
        info!("Captured {} total unique items successfully", all_captures.len());
        for cap in &all_captures {
            if let Some(secret) = cap.get("secret").and_then(Value::as_str) {
                if let Some(version) = cap.get("version").and_then(Value::as_i64) {
                    let source = cap.get("source").and_then(Value::as_str).unwrap_or("hook");
                    info!("Secret({}): {} (source: {})", version, secret, source);
                }
            }
        }
    }

    Ok(all_captures)
}

