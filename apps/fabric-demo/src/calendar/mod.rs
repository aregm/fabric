//! The calendar service (section 5.1; tasks T4.1 and T4.2). Each person has one. It holds that
//! person's encrypted calendar, answers agents only with free/busy and only for a valid token,
//! and shows titles only to the owner's dashboard.

pub mod client;
pub mod fixtures;
pub mod freebusy;
pub mod service;
pub mod store;

pub use client::{CalendarClient, CallError};
pub use freebusy::{Busy, free_busy};
pub use service::{Calendar, CalendarConfig, CalendarParts, TOKEN_SCHEME, bind, router, serve};
pub use store::{AddedBy, Event, Store, StoreError, StoreKey};
