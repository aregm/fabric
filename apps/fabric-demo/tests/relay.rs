//! The relay (task T5.1): FIFO mailboxes, waiting reads, 10-minute retention, and an audit log
//! with nothing but mailbox, size and time.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use fabric_demo::audit::AuditLog;
use fabric_demo::clock::ManualClock;
use fabric_demo::net::bind_loopback;
use fabric_demo::relay::{RETENTION_MS, Relay, RelayClient, new_mailbox, serve};
use fabric_policy::time::parse_utc;

fn scratch(test: &str) -> PathBuf {
    let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("relay")
        .join(test);
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).unwrap();
    directory
}

struct Running {
    client: RelayClient,
    clock: Arc<ManualClock>,
    mailboxes: [String; 2],
    audit: PathBuf,
    server: tokio::task::JoinHandle<std::io::Result<()>>,
}

async fn start(test: &str) -> Running {
    let directory = scratch(test);
    let mailboxes = [new_mailbox().unwrap(), new_mailbox().unwrap()];
    let clock = Arc::new(ManualClock::new(parse_utc("2027-01-11T08:00Z").unwrap()));
    let audit = directory.join("relay.jsonl");
    let relay = Relay::new(&mailboxes, clock.clone(), AuditLog::open(&audit).unwrap());
    let listener = bind_loopback("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(serve(listener, relay, std::future::pending()));
    Running {
        client: RelayClient::new(&url),
        clock,
        mailboxes,
        audit,
        server,
    }
}

#[tokio::test]
async fn mailboxes_keep_order_and_each_reader_keeps_its_place() {
    let relay = start("order").await;
    let [inbox, other] = &relay.mailboxes;
    for frame in [b"one".as_slice(), b"two", b"three"] {
        relay.client.post(inbox, frame).await.unwrap();
    }
    relay.client.post(other, b"elsewhere").await.unwrap();

    let all = relay.client.poll(inbox, 0, 0).await.unwrap();
    let frames: Vec<&[u8]> = all.iter().map(|frame| frame.frame.as_slice()).collect();
    assert_eq!(frames, [b"one".as_slice(), b"two", b"three"]);
    assert_eq!(
        all.iter().map(|frame| frame.seq).collect::<Vec<_>>(),
        [1, 2, 3]
    );
    let later = relay.client.poll(inbox, 2, 0).await.unwrap();
    assert_eq!(later.len(), 1);
    assert_eq!(later[0].frame, b"three");
    assert_eq!(
        relay.client.poll(inbox, 0, 0).await.unwrap().len(),
        3,
        "reading removes nothing"
    );

    let unknown = new_mailbox().unwrap();
    assert!(relay.client.post(&unknown, b"x").await.is_err());
    assert!(relay.client.poll(&unknown, 0, 0).await.is_err());
    assert!(
        relay.client.post(inbox, &vec![0; 70_000]).await.is_err(),
        "too long"
    );
    relay.server.abort();
}

#[tokio::test]
async fn a_waiting_read_returns_when_a_frame_arrives() {
    let relay = start("wait").await;
    let inbox = relay.mailboxes[0].clone();
    let reader = relay.client.clone();
    let waiting = tokio::spawn({
        let inbox = inbox.clone();
        async move { reader.poll(&inbox, 0, 10_000).await }
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    relay.client.post(&inbox, b"wake up").await.unwrap();
    let frames = tokio::time::timeout(Duration::from_secs(5), waiting)
        .await
        .expect("the read returns well before its timeout")
        .unwrap()
        .unwrap();
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].frame, b"wake up");

    let started = std::time::Instant::now();
    assert!(relay.client.poll(&inbox, 1, 300).await.unwrap().is_empty());
    assert!(started.elapsed() >= Duration::from_millis(250));
    relay.server.abort();
}

#[tokio::test]
async fn frames_expire_after_ten_minutes_and_the_log_holds_only_metadata() {
    let relay = start("retention").await;
    let inbox = &relay.mailboxes[0];
    relay.client.post(inbox, b"secret words").await.unwrap();
    relay.clock.advance(RETENTION_MS - 1);
    relay.client.post(inbox, b"later words").await.unwrap();
    assert_eq!(relay.client.poll(inbox, 0, 0).await.unwrap().len(), 2);
    relay.clock.advance(1);
    let left = relay.client.poll(inbox, 0, 0).await.unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].frame, b"later words");

    let log = fs::read_to_string(&relay.audit).unwrap();
    assert_eq!(log.lines().count(), 2);
    for line in log.lines() {
        let record: serde_json::Value = serde_json::from_str(line).unwrap();
        let mut keys: Vec<&str> = record
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(keys, ["at_ms", "bytes", "component", "mailbox", "seq"]);
    }
    assert!(!log.contains("words"));
    relay.server.abort();
}
