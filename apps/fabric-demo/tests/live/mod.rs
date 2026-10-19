//! A whole demo in one process for the live tests: one shared Isabelle server, the relay, and for
//! Alice and Bob a calendar service, a trusted host with a policy gate, and an agent.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use fabric_demo::agent::{Agent, HttpWorld, Planner, Scripted};
use fabric_demo::audit::AuditLog;
use fabric_demo::calendar::{
    self, Calendar, CalendarClient, CalendarParts, Store, StoreKey, fixtures,
};
use fabric_demo::clock::{Clock, DemoClock};
use fabric_demo::host::{
    self, Gateway, GatewayParts, Host, HostClient, HostParts, PolicyConfig, PolicyGate, RequestForm,
};
use fabric_demo::net::bind_loopback;
use fabric_demo::relay::{self, Relay, RelayClient, new_mailbox};
use fabric_policy::server::{Server, ServerOptions};
use fabric_policy::time::parse_utc;
use fabric_policy::{GateConfig, Role};
use fabric_token::{BindingKey, MemoryStore, PersonKeys};

pub const OWNER_SECRET: &str = "owner-secret-for-live-tests-0123456789";
pub const AGENT_SECRET: &str = "agent-secret-for-live-tests-0123456789";

pub fn repository() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the app lives two levels below the repository root")
}

pub struct Person {
    pub name: &'static str,
    pub host: Arc<Host>,
    pub host_url: String,
    pub calendar: Arc<Calendar>,
    pub calendar_url: String,
}

impl Person {
    /// This person's agent, registered with the host like the `agent` command does.
    pub async fn agent(
        &self,
        planner: Planner,
        audit: AuditLog,
        clock: Arc<dyn Clock>,
    ) -> Agent<HttpWorld> {
        let world = HttpWorld {
            host: HostClient::new(&self.host_url, AGENT_SECRET),
            calendar: CalendarClient::new(&self.calendar_url),
            key: BindingKey::generate().unwrap(),
            clock,
        };
        world
            .host
            .register(&world.key.verifying_key())
            .await
            .unwrap();
        Agent::new(world, planner, self.name, audit)
    }

    pub fn owner_events(&self) -> Vec<fabric_demo::calendar::Event> {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current()
                .block_on(CalendarClient::new(&self.calendar_url).owner_events(OWNER_SECRET))
        })
        .unwrap()
    }
}

pub struct Demo {
    pub directory: PathBuf,
    pub clock: Arc<dyn Clock>,
    pub alice: Person,
    pub bob: Person,
    pub relay: RelayClient,
    pub inboxes: [String; 2],
    server: Server,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl Drop for Demo {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
        self.alice.host.stop();
        self.bob.host.stop();
        self.server.shutdown();
    }
}

async fn serve_router(
    router: axum::Router,
    tasks: &mut Vec<tokio::task::JoinHandle<()>>,
) -> String {
    let listener = bind_loopback("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tasks.push(tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    }));
    url
}

impl Demo {
    /// Starts everything. Isabelle's sessions take a little while to open.
    pub async fn start(test: &str) -> Self {
        let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(test);
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let clock: Arc<dyn Clock> = Arc::new(DemoClock::starting_now(
            parse_utc("2027-01-11T08:00Z").unwrap(),
        ));

        let isabelle = GateConfig::for_repository(repository());
        let server = Server::start(&ServerOptions {
            isabelle_home: &isabelle.isabelle_home,
            user_home: &isabelle.user_home,
            name: test,
            log_dir: &directory,
        })
        .expect("the Isabelle server starts");
        let address = server.address();

        let mut tasks = Vec::new();
        let inboxes = [new_mailbox().unwrap(), new_mailbox().unwrap()];
        let relay = Relay::new(
            &inboxes,
            clock.clone(),
            AuditLog::open(&directory.join("relay.jsonl")).unwrap(),
        );
        let relay_url = serve_router(relay::router(relay), &mut tasks).await;

        let keys = [
            PersonKeys::load_or_create(&MemoryStore::new(), "alice").unwrap(),
            PersonKeys::load_or_create(&MemoryStore::new(), "bob").unwrap(),
        ];
        let mut people = Vec::new();
        for (index, (name, peer, role, peer_name)) in [
            ("alice", "bob", Role::Coordinator, "Bob"),
            ("bob", "alice", Role::Invitee, "Alice"),
        ]
        .into_iter()
        .enumerate()
        {
            let own = &keys[index];
            let other = &keys[1 - index];
            let events = fixtures::load(
                &repository()
                    .join("demo")
                    .join("fixtures")
                    .join(format!("{name}.json")),
                name,
            )
            .unwrap();
            let store = Store::open_or_create(
                &directory.join(format!("{name}.cal")),
                name,
                StoreKey::from_bytes([index as u8 + 1; 32]),
                &events,
            )
            .unwrap();
            let calendar = Calendar::new(CalendarParts {
                person: name.to_owned(),
                peer_name: peer_name.to_owned(),
                gate_key: own.public().gate().unwrap(),
                owner_secret: OWNER_SECRET.to_owned(),
                store,
                fixtures: events,
                clock: clock.clone(),
                audit: AuditLog::open(&directory.join(format!("{name}-calendar.jsonl"))).unwrap(),
            });
            let calendar_url = serve_router(calendar::router(calendar.clone()), &mut tasks).await;

            let gateway = Gateway::new(GatewayParts {
                person: name.to_owned(),
                peer: peer.to_owned(),
                role,
                negotiation: (role == Role::Coordinator)
                    .then(|| host::new_negotiation_id().unwrap()),
                identity: own.identity().clone(),
                noise_secret: own.noise_secret(),
                own_keys: own.public(),
                peer_keys: other.public(),
                relay: RelayClient::new(&relay_url),
                inbox: inboxes[index].clone(),
                peer_inbox: inboxes[1 - index].clone(),
                clock: clock.clone(),
                audit: AuditLog::open(&directory.join(format!("{name}-gateway.jsonl"))).unwrap(),
            })
            .unwrap();
            let policy = PolicyGate::connect(
                &PolicyConfig {
                    repository: repository().to_owned(),
                    isabelle: address.clone(),
                    work_dir: directory.join(format!("{name}-checks")),
                },
                own.minter(),
            )
            .expect("the policy gate opens its session");
            let host = Host::new(HostParts {
                gateway: gateway.clone(),
                policy: Some(policy),
                agent_secret: AGENT_SECRET.to_owned(),
                owner_secret: OWNER_SECRET.to_owned(),
                clock: clock.clone(),
                audit: AuditLog::open(&directory.join(format!("{name}-host.jsonl"))).unwrap(),
            });
            tasks.push(tokio::spawn(gateway.run_inbound()));
            let host_url = serve_router(host::router(host.clone()), &mut tasks).await;
            people.push(Person {
                name,
                host,
                host_url,
                calendar,
                calendar_url,
            });
        }
        let bob = people.pop().unwrap();
        let alice = people.pop().unwrap();
        Self {
            directory,
            clock,
            alice,
            bob,
            relay: RelayClient::new(&relay_url),
            inboxes,
            server,
            tasks,
        }
    }

    /// Both owners file the scenario's requests (section 14).
    pub fn file_requests(&self) {
        let form = |peer: &str, preference: &str| RequestForm {
            peer: peer.to_owned(),
            duration_minutes: 30,
            window_start: "2027-01-11T08:00Z".to_owned(),
            window_end: "2027-01-18T08:00Z".to_owned(),
            expires_at: "2027-01-12T08:00Z".to_owned(),
            preference: preference.to_owned(),
        };
        self.alice
            .host
            .file_request(&form("bob", "Weekday afternoons work best."))
            .unwrap();
        self.bob
            .host
            .file_request(&form("alice", "Any weekday works."))
            .unwrap();
    }

    pub fn audit(&self, name: &str) -> AuditLog {
        AuditLog::open(&self.directory.join(format!("{name}.jsonl"))).unwrap()
    }
}

/// Steps both agents until `done` holds or `limit` passes.
pub async fn run_until(
    alice: &mut Agent<HttpWorld>,
    bob: &mut Agent<HttpWorld>,
    limit: Duration,
    mut done: impl FnMut() -> bool,
) -> bool {
    let started = Instant::now();
    while started.elapsed() < limit {
        if done() {
            return true;
        }
        alice.step(200).await.expect("alice's step");
        bob.step(200).await.expect("bob's step");
    }
    done()
}

pub fn scripted() -> Planner {
    Planner::Scripted(Scripted::Rules)
}
