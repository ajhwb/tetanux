use clap::Parser;

#[derive(Parser)]
#[command(name = "tetanux")]
#[command(version = "1.0")]
#[command(about = "A proxy server based on tinyproxy", long_about = None)]
pub struct Cli {
    #[arg(short)]
    /// Configuration file
    pub c: Option<String>,
}