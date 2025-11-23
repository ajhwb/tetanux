use clap::Parser;

#[derive(Parser)]
#[command(name = "tetanux")]
#[command(version = "1.0")]
#[command(about = "A small web proxy server", long_about = None)]
pub struct Cli {
    #[arg(short)]
    /// Configuration file
    pub c: Option<String>,
}