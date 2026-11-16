//! The Tauri host of Fabric Calendar (work package CD-W02).
//!
//! The host starts one broker child over private stdin and stdout pipes and relays the
//! `fabric-desktop/1` owner channel between the webview and the broker. The webview reaches the
//! host through three commands and nothing else (`capabilities/owner-interface.json`):
//!
//! - `owner_attach` gives the host the channel on which it sends states and frames;
//! - `owner_send` passes one owner request frame to the broker, after the host has validated it;
//! - `broker_restart` starts the broker again after it died, when the owner asks.
//!
//! The host holds no Google credential and no database, and makes no policy decision. The
//! relaying itself is the crate `fabric-desktop-relay`, which has no Tauri in it.

mod bridge;
mod host;

use fabric_desktop_relay::broker_beside;
use tauri::ipc::Channel;
use tauri::{Manager, RunEvent, State};

use crate::bridge::{HostMessage, SendRefusal};
use crate::host::{Host, Trace};

#[tauri::command]
fn owner_attach(host: State<'_, Host>, channel: Channel<HostMessage>) {
    host.attach(Box::new(move |message| {
        // A send fails only when the webview is gone; the next attach replaces the channel.
        let _ = channel.send(message);
    }));
}

// Off the main thread: writing to the broker's pipe can wait for the broker to read.
#[tauri::command(async)]
fn owner_send(host: State<'_, Host>, frame: String) -> Result<(), SendRefusal> {
    host.send(frame.as_bytes())
}

#[tauri::command(async)]
fn broker_restart(host: State<'_, Host>) -> Result<(), String> {
    host.restart()
}

fn main() {
    let application = tauri::Builder::default()
        .manage(Host::new(Trace::from_environment()))
        .invoke_handler(tauri::generate_handler![
            owner_attach,
            owner_send,
            broker_restart
        ])
        .setup(|application| {
            // The lock lives in the application's own directory under Application Support.
            let lock_directory = application.path().app_data_dir()?;
            // The broker ships beside the host executable, in Contents/MacOS of the bundle.
            let broker = broker_beside(&std::env::current_exe()?);
            application.state::<Host>().start(&lock_directory, &broker);
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("the application could not be built");
    application.run(|handle, event| {
        if let RunEvent::Exit = event {
            handle.state::<Host>().quit();
        }
    });
}
