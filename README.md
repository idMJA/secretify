# secretify

A secrets scraper that monitors and extracts secrets.

Original method by [misiektoja/spotify_monitor](https://github.com/misiektoja/spotify_monitor/blob/dev/debug/spotify_monitor_secret_grabber.py), converted to Rust.

## Installation

### Prerequisites

- Rust 1.70+ (install from [rustup.rs](https://rustup.rs))
- Chrome/Chromium browser installed

To build the project:

```bash
cargo build --release
```

## Usage

To run:

```bash
cargo run --release
```

Or directly execute the binary:

```bash
./target/release/secretify
```

## Using the JSON Data

The scraper generates three JSON files that contain extracted secrets:

### Plain Secrets (array)

**File**: `secrets/secrets.json`

Returns a JSON array of following objects: `{ "version": number, "secret": string }`

```json
[
  { "version": 59, "secret": "{iOFn;4}<1PFYKPV?5{%u14]M>/V0hDH" },
  { "version": 60, "secret": "OmE{ZA.J^\":0FG\\Uz?[@WW" },
  { "version": 61, "secret": ",7/*F(\"rLJ2oxaKL^f+E1xvP@N" }
]
```

### Secret Bytes (array)

**File**: `secrets/secretBytes.json`

Returns a JSON array of following objects: `{ "version": number, "secret": number[] }`

```json
[
  { "version": 59, "secret": [123, 105, 79, 70, 110, 59, 52, 125] },
  { "version": 60, "secret": [79, 109, 69, 123, 90, 65, 46, 74] },
  { "version": 61, "secret": [44, 55, 47, 42, 70, 40, 34, 114] }
]
```

### Secret Bytes (object/dict)

**File**: `secrets/secretDict.json`

Returns a JSON object mapping each version to its array of byte values: `{ [version: string]: number[] }`

```json
{
  "59": [123, 105, 79, 70, 110, 59, 52, 125],
  "60": [79, 109, 69, 123, 90, 65, 46, 74],
  "61": [44, 55, 47, 42, 70, 40, 34, 114]
}
```

### Rust Types

The scraper outputs secrets in one of two unified formats:

**Array format** (`secrets.json`, `secretBytes.json`):

```rust
#[derive(Serialize, Deserialize)]
struct Secrets {
    version: i32,
    secret: T, // String or Vec<i32>
}
```

**Object/Dict format** (`secretDict.json`):

```rust
type SecretsDict = BTreeMap<String, Vec<i32>>;
```

### Usage Example (Rust)

```rust
use serde_json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Fetch secrets
    let response = reqwest::Client::new()
        .get("https://github.com/idMJA/secretify/blob/master/secrets/secrets.json?raw=true")
        .send()
        .await?;
    
    let secrets: Vec<serde_json::Value> = response.json().await?;
    
    // Get latest version
    if let Some(latest) = secrets.last() {
        let version = latest.get("version").and_then(|v| v.as_i64());
        let secret = latest.get("secret").and_then(|s| s.as_str());
        println!("Version {:?}: {:?}", version, secret);
    }
    
    Ok(())
}
```

### Usage Example (TypeScript/JavaScript)

```typescript
// Fetch secrets
const response = await fetch('https://github.com/idMJA/secretify/blob/master/secrets/secrets.json?raw=true');
const secrets = await response.json();

// Get latest version
const latestSecret = secrets[secrets.length - 1];
console.log(`Version ${latestSecret.version}: ${latestSecret.secret}`);
```

### Usage Example (Python)

```python
import requests

# Fetch secrets
response = requests.get("https://github.com/idMJA/secretify/blob/master/secrets/secretDict.json?raw=true")
secrets = response.json()

# Get latest version
latest_secret = secrets[(v := max(secrets, key=int))]
print(f"Version {v}: {latest_secret}")
```

## How It Works

1. Launches a headless Chrome browser with stealth mode to avoid detection
2. Navigates to `https://open.spotify.com`
3. Injects a hook into `Object.prototype.secret` to capture secret assignments
4. Waits for secrets to be captured during page load
5. Extracts and processes the captured secrets
6. Exports data in three JSON formats for easy consumption

## Project Structure

```
src/
├── main.rs                 # Entry point
└── secrets/
    ├── mod.rs             # Module definitions
    ├── grabber.rs         # Browser automation & secret extraction
    ├── summarizer.rs      # Data processing & formatting
    ├── models.rs          # Data structures
    └── utils.rs           # Helper functions
```

## License

This project is provided as-is for research and educational purposes only.
