use std::net::SocketAddr;

use mouseshare_net::{connect, listen, PeerInfo};
use mouseshare_protocol::Message;

#[tokio::test]
async fn handshake_and_bidirectional_message_roundtrip() {
    let listener = listen("127.0.0.1:0".parse::<SocketAddr>().unwrap())
        .await
        .expect("bind listener");
    let addr = listener.local_addr().expect("local_addr");

    let server_task = tokio::spawn(async move {
        let mut conn = listener.accept().await.expect("accept");
        let peer = conn
            .handshake_as_listener("listener-screen".to_string(), 1920, 1080)
            .await
            .expect("listener handshake");

        // Receive a batch of messages from the dialer.
        let mut received = Vec::new();
        for _ in 0..4 {
            received.push(conn.recv().await.expect("recv from dialer"));
        }

        // Send a batch back to the dialer, in a specific order.
        let to_send = [
            Message::MouseMove { dx: 100, dy: -100 },
            Message::Heartbeat,
            Message::MouseMove { dx: 0, dy: 1 },
        ];
        for msg in &to_send {
            conn.send(msg).await.expect("send to dialer");
        }

        (peer, received)
    });

    let mut dialer_conn = connect(addr).await.expect("connect");
    let dialer_peer: PeerInfo = dialer_conn
        .handshake_as_dialer("dialer-screen".to_string(), 640, 480)
        .await
        .expect("dialer handshake");

    assert_eq!(dialer_peer.screen_id, "listener-screen");
    assert_eq!(dialer_peer.width, 1920);
    assert_eq!(dialer_peer.height, 1080);

    // Send a batch of messages to the listener, in a specific order.
    let dialer_to_send = [
        Message::MouseMove { dx: 1, dy: 2 },
        Message::MouseMove { dx: -3, dy: 4 },
        Message::Heartbeat,
        Message::MouseMove { dx: 5, dy: -6 },
    ];
    for msg in &dialer_to_send {
        dialer_conn.send(msg).await.expect("send to listener");
    }

    // Receive the listener's batch.
    let mut from_listener = Vec::new();
    for _ in 0..3 {
        from_listener.push(dialer_conn.recv().await.expect("recv from listener"));
    }

    let (listener_peer, received_by_listener) = server_task.await.expect("server task join");

    assert_eq!(listener_peer.screen_id, "dialer-screen");
    assert_eq!(listener_peer.width, 640);
    assert_eq!(listener_peer.height, 480);

    // Exact content and order, both directions.
    assert_eq!(received_by_listener, dialer_to_send.to_vec());
    assert_eq!(
        from_listener,
        vec![
            Message::MouseMove { dx: 100, dy: -100 },
            Message::Heartbeat,
            Message::MouseMove { dx: 0, dy: 1 },
        ]
    );
}
