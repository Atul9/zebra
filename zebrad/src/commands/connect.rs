//! `connect` subcommand - test stub for talking to zcashd

use crate::{
    error::{Error, ErrorKind},
    prelude::*,
};

use abscissa_core::{Command, Options, Runnable};

use futures::prelude::*;

/// `connect` subcommand
#[derive(Command, Debug, Options)]
pub struct ConnectCmd {
    /// The address of the node to connect to.
    #[options(
        help = "The address of the node to connect to.",
        default = "127.0.0.1:8233"
    )]
    addr: std::net::SocketAddr,
}

impl Runnable for ConnectCmd {
    /// Start the application.
    fn run(&self) {
        info!(connect.addr = ?self.addr);

        use crate::components::tokio::TokioComponent;
        let rt = app_writer()
            .state_mut()
            .components
            .get_downcast_mut::<TokioComponent>()
            .expect("TokioComponent should be available")
            .rt
            .take();

        rt.expect("runtime should not already be taken")
            .block_on(self.connect())
            // Surface any error that occurred executing the future.
            .unwrap();
    }
}

impl ConnectCmd {
    async fn connect(&self) -> Result<(), Error> {
        use zebra_network::{Request, Response};

        info!("begin tower-based peer handling test stub");
        use tower::{buffer::Buffer, service_fn, Service, ServiceExt};

        let node = Buffer::new(
            service_fn(|req| async move {
                info!(?req);
                Ok::<Response, Error>(Response::Nil)
            }),
            1,
        );

        let mut config = app_config().network.clone();
        // Use a different listen addr so that we don't conflict with another local node.
        config.listen_addr = "127.0.0.1:38233".parse().unwrap();
        // Connect only to the specified peer.
        config.initial_mainnet_peers = vec![self.addr.to_string()];

        let (mut peer_set, _address_book) = zebra_network::init(config, node).await;
        let mut retry_peer_set =
            tower::retry::Retry::new(zebra_network::RetryErrors, peer_set.clone());

        info!("waiting for peer_set ready");
        peer_set
            .ready_and()
            .await
            .map_err(|e| Error::from(ErrorKind::Io.context(e)))?;

        info!("peer_set became ready");

        use futures::stream::{FuturesUnordered, StreamExt};
        use std::collections::BTreeSet;
        use zebra_chain::block::BlockHeaderHash;
        use zebra_chain::types::BlockHeight;

        // genesis
        let mut tip = BlockHeaderHash([
            8, 206, 61, 151, 49, 176, 0, 192, 131, 56, 69, 92, 138, 74, 107, 208, 93, 161, 110, 38,
            177, 29, 170, 27, 145, 113, 132, 236, 232, 15, 4, 0,
        ]);

        let mut downloaded_block_heights = BTreeSet::<BlockHeight>::new();
        downloaded_block_heights.insert(BlockHeight(0));
        let mut block_requests = FuturesUnordered::new();
        let mut requested_block_heights = 0;
        while requested_block_heights < 700_000 {
            // Request the next 500 hashes.
            retry_peer_set.ready_and().await.unwrap();
            let hashes = if let Ok(Response::BlockHeaderHashes(hashes)) = retry_peer_set
                .call(Request::FindBlocks {
                    known_blocks: vec![tip],
                    stop: None,
                })
                .await
            {
                info!(
                    new_hashes = hashes.len(),
                    requested = requested_block_heights,
                    in_flight = block_requests.len(),
                    downloaded = downloaded_block_heights.len(),
                    highest = downloaded_block_heights.iter().next_back().unwrap().0,
                    "requested more hashes"
                );
                requested_block_heights += hashes.len();
                hashes
            } else {
                panic!("request failed, TODO implement retry");
            };

            tip = *hashes.last().unwrap();

            // Request the corresponding blocks in chunks
            for chunk in hashes.chunks(10usize) {
                peer_set.ready_and().await.unwrap();
                block_requests
                    .push(peer_set.call(Request::BlocksByHash(chunk.iter().cloned().collect())));
            }

            // Allow at most 300 block requests in flight.
            while block_requests.len() > 300 {
                match block_requests.next().await {
                    Some(Ok(Response::Blocks(blocks))) => {
                        for block in &blocks {
                            downloaded_block_heights.insert(block.coinbase_height().unwrap());
                        }
                    }
                    Some(Err(e)) => {
                        error!(%e);
                    }
                    _ => continue,
                }
            }
        }

        while let Some(Ok(Response::Blocks(blocks))) = block_requests.next().await {
            for block in &blocks {
                downloaded_block_heights.insert(block.coinbase_height().unwrap());
            }
        }

        let eternity = future::pending::<()>();
        eternity.await;

        Ok(())
    }
}
