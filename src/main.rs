use jsonrpsee::server::ServerBuilder;
use eyre::Result;
use revm_passthrough_proxy::rpc::{PassthroughProxy, PassthroughApiServer};
use std::env;

#[tokio::main]
async fn main() -> Result<()> {
    // Replace with your actual endpoint URL
    let endpoint = env::var("RPC").expect("env var RPC must be set");

    // Initialize the PassthroughProxy
    let proxy = PassthroughProxy::init(&endpoint)?;

    // Build the server on the desired address and port
    let server = ServerBuilder::default()
        .build("127.0.0.1:8545")
        .await?;

    // Start the server with your RPC methods
    let handle = server.start(proxy.into_rpc());

    println!("Server started at 127.0.0.1:8545");

    // Keep the server running indefinitely
    handle.stopped().await;

    Ok(())
}
