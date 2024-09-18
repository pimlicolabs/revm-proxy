use eyre::Result;
use jsonrpsee::server::ServerBuilder;
use revm_passthrough_proxy::rpc::{PassthroughApiServer, PassthroughProxy};
use std::env;

#[tokio::main]
async fn main() -> Result<()> {
    // Replace with your actual endpoint URL
    let endpoint = env::var("RPC").expect("env var RPC must be set");

    let port = env::var("PORT").expect("env var PORT must be set");

    let chain_id: u64 = env::var("CHAIN_ID")
        .expect("env var CHAIN_ID must be set")
        .parse()
        .expect("CHAIN_ID must be a valid u64");

    // Initialize the PassthroughProxy
    let proxy = PassthroughProxy::init(&endpoint, chain_id)?;

    let server = ServerBuilder::default()
        .build(format!("0.0.0.0:{}", port))
        .await?;

    // Start the server with your RPC methods
    let handle = server.start(proxy.into_rpc());

    println!("Server started at 0.0.0.0:{}", port);

    // Keep the server running indefinitely
    handle.stopped().await;

    Ok(())
}
