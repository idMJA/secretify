mod secrets;

use secrets::grabber::grab_live;
use secrets::summarizer::summarise;
use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    tracing::info!("Starting secrets extractor...");

    match grab_live().await {
        Ok(caps) => {
            tracing::info!("Successfully captured {} secrets", caps.len());
            summarise(&caps)?;
        }
        Err(e) => {
            tracing::error!("Error grabbing live secrets: {}", e);
            std::process::exit(1);
        }
    }

    Ok(())
}
