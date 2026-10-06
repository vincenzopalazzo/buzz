fn main() {
    if let Err(e) = buzz_lightning_mcp::run() {
        eprintln!("buzz-lightning-mcp: {e}");
        std::process::exit(1);
    }
}
