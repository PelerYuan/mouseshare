use std::net::SocketAddr;

use mouseshare_net::{listen, NetError};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

/// A peer that writes only part of the 4-byte length prefix and then
/// disconnects must surface as a clean `Err(NetError::ConnectionClosed)`
/// from `recv()` — not a panic and not a hang.
#[tokio::test]
async fn mid_frame_disconnect_is_clean_error() {
    let listener = listen("127.0.0.1:0".parse::<SocketAddr>().unwrap())
        .await
        .expect("bind listener");
    let addr = listener.local_addr().expect("local_addr");

    let server_task = tokio::spawn(async move {
        let mut conn = listener.accept().await.expect("accept");
        conn.recv().await
    });

    let mut client = TcpStream::connect(addr).await.expect("client connect");
    // Write only 2 of the 4 length-prefix bytes, then drop the socket.
    client.write_all(&[0x00, 0x00]).await.expect("partial write");
    drop(client);

    let result = tokio::time::timeout(std::time::Duration::from_secs(5), server_task)
        .await
        .expect("recv() hung instead of erroring")
        .expect("server task panicked");

    assert!(
        matches!(result, Err(NetError::ConnectionClosed)),
        "expected Err(NetError::ConnectionClosed), got {:?}",
        result
    );
}
