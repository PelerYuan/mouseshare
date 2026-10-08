use std::net::SocketAddr;

use mouseshare_net::{connect, listen, NetError, PairingCode, PeerInfo};
use mouseshare_protocol::{Message, MonitorInfo};

fn info(id: &str, w: i32, h: i32) -> PeerInfo {
    PeerInfo {
        device_id: id.to_string(),
        monitors: vec![MonitorInfo {
            name: "M".into(),
            x: 0,
            y: 0,
            width: w,
            height: h,
            primary: true,
        }],
    }
}

#[tokio::test]
async fn handshake_and_bidirectional_message_roundtrip() {
    let code = PairingCode::generate();
    let listener_code = code.clone();
    let listener = listen("127.0.0.1:0".parse::<SocketAddr>().unwrap())
        .await
        .expect("bind listener");
    let addr = listener.local_addr().expect("local_addr");

    let server_task = tokio::spawn(async move {
        let mut conn = listener.accept().await.expect("accept");
        let peer = conn
            .handshake_as_listener(&listener_code, &info("listener-screen", 1920, 1080))
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
            Message::Ping(7),
            Message::MouseMove { dx: 0, dy: 1 },
        ];
        for msg in &to_send {
            conn.send(msg).await.expect("send to dialer");
        }

        (peer, received)
    });

    let mut dialer_conn = connect(addr).await.expect("connect");
    let dialer_peer: PeerInfo = dialer_conn
        .handshake_as_dialer(&code, &info("dialer-screen", 640, 480))
        .await
        .expect("dialer handshake");

    assert_eq!(dialer_peer, info("listener-screen", 1920, 1080));

    // Send a batch of messages to the listener, in a specific order.
    let dialer_to_send = [
        Message::MouseMove { dx: 1, dy: 2 },
        Message::MouseMove { dx: -3, dy: 4 },
        Message::Ping(7),
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

    assert_eq!(listener_peer, info("dialer-screen", 640, 480));

    // Exact content and order, both directions.
    assert_eq!(received_by_listener, dialer_to_send.to_vec());
    assert_eq!(
        from_listener,
        vec![
            Message::MouseMove { dx: 100, dy: -100 },
            Message::Ping(7),
            Message::MouseMove { dx: 0, dy: 1 },
        ]
    );
}

#[tokio::test]
async fn wrong_pairing_code_is_rejected_on_both_sides() {
    let listener = listen("127.0.0.1:0".parse::<SocketAddr>().unwrap())
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let listener_code = PairingCode::generate();
    let dialer_code = PairingCode::generate();
    assert_ne!(listener_code, dialer_code);

    let server = tokio::spawn(async move {
        let mut conn = listener.accept().await.unwrap();
        conn.handshake_as_listener(&listener_code, &info("l", 100, 100))
            .await
    });

    let mut dialer = connect(addr).await.unwrap();
    let dialer_result = dialer
        .handshake_as_dialer(&dialer_code, &info("d", 100, 100))
        .await;
    let listener_result = server.await.unwrap();

    assert!(matches!(dialer_result, Err(NetError::AuthFailed)));
    assert!(matches!(listener_result, Err(NetError::AuthFailed)));
}

#[tokio::test]
async fn large_clipboard_payload_survives_encryption() {
    let code = PairingCode::generate();
    let listener = listen("127.0.0.1:0".parse::<SocketAddr>().unwrap())
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let c2 = code.clone();
    let big = "x".repeat(512 * 1024);
    let expected = big.clone();
    let server = tokio::spawn(async move {
        let mut conn = listener.accept().await.unwrap();
        conn.handshake_as_listener(&c2, &info("l", 1, 1)).await.unwrap();
        conn.recv().await.unwrap()
    });
    let mut dialer = connect(addr).await.unwrap();
    dialer
        .handshake_as_dialer(&code, &info("d", 1, 1))
        .await
        .unwrap();
    dialer.send(&Message::ClipboardText(big)).await.unwrap();
    assert_eq!(server.await.unwrap(), Message::ClipboardText(expected));
}
