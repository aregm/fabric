//! The processes of the calendar demo (docs/isabelle-calendar-demo.md, section 5): the calendar
//! service (Phase 4), the relay and the trusted host's messaging gateway (Phase 5). The agents,
//! dashboard and launcher follow.

pub mod agent;
pub mod api;
pub mod audit;
pub mod calendar;
pub mod client;
pub mod clock;
pub mod env;
pub mod host;
pub mod net;
pub mod relay;
