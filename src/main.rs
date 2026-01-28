// SPDX-License-Identifier: MIT
//
// Author: Johannes Leupolz <dev@leupolz.eu>

// apt install libdbus-1-dev pkg-config libpam0g-dev

use std::fs::OpenOptions;
use std::io;
use std::os::fd::AsRawFd;
use std::time;
use std::time::Duration;

use dbus::blocking::Connection;
use log::{debug, error, info, warn};

use std::thread;

use crate::pam::NoPasswordClient;
mod pam;

// see include/uapi/linux/kd.h
const KDGKBMODE: u64 = 0x4B44; // gets current keyboard mode
const K_OFF: u64 = 0x04;

fn start_pam_session() -> anyhow::Result<String> {
    let mut client = NoPasswordClient::new_client("fallbackdm").expect("Failed to init PAM client.");

    // Actually try to authenticate:
    client.authenticate().expect("Authentication failed!");
    // Now that we are authenticated, it's possible to open a sesssion:
    client.open_session().expect("Failed to open a session!");

    let session_id= client.get_env("XDG_SESSION_ID")?.expect("XDG_SESSION_ID is empty");

    Ok(session_id)
}

fn connect_to_dbus() -> anyhow::Result<Connection> {
    let conn = Connection::new_system().expect("failed to connect to system bus");

    Ok(conn)
}

fn send_take_control_message(conn: &Connection, session: &str) -> anyhow::Result<()> {
    // https://www.freedesktop.org/software/systemd/man/latest/org.freedesktop.login1.html

    // can be found via
    // busctl --system list
    // busctl introspect --system org.freedesktop.Notifications
    let node = format!("/org/freedesktop/login1/session/{}", session);

    // create a wrapper struct around the connection
    let proxy = conn.with_proxy("org.freedesktop.login1", &node, Duration::from_millis(5000));

    // introspect for debugging
    let (xml,): (String,) =
        proxy.method_call("org.freedesktop.DBus.Introspectable", "Introspect", ())?;

    debug!("introspect dbus node {}: {}", &node, xml);

    // Now make the method call. The ListNames method call takes zero input parameters and
    // one output parameter which is an array of strings.
    // Therefore the input is a zero tuple "()", and the output is a single tuple "(names,)".
    let (): () = proxy.method_call("org.freedesktop.login1.Session", "TakeControl", (false,))?;

    Ok(())
}

fn send_release_control_message(conn: &Connection, session: &str) -> anyhow::Result<()> {
    // https://www.freedesktop.org/software/systemd/man/latest/org.freedesktop.login1.html

    // can be found via
    // busctl --system list
    // busctl introspect --system org.freedesktop.Notifications
    let node = format!("/org/freedesktop/login1/session/{}", session);

    // create a wrapper struct around the connection
    let proxy = conn.with_proxy("org.freedesktop.login1", &node, Duration::from_millis(5000));

    // Now make the method call. The ListNames method call takes zero input parameters and
    // one output parameter which is an array of strings.
    // Therefore the input is a zero tuple "()", and the output is a single tuple "(names,)".
    let (): () = proxy.method_call("org.freedesktop.login1.Session", "ReleaseControl", ())?;

    Ok(())
}

fn check_vt_status() {
    match OpenOptions::new().read(true).open("/dev/tty1") {
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            info!("/dev/tty1 not present — no VT-related input problem");
        }
        Err(err) => {
            error!("failed to open /dev/tty1: {}", err);
        }
        Ok(tty) => {
            let fd = tty.as_raw_fd();
            let mut mode: u64 = 0;

            let rc = unsafe { libc::ioctl(fd, KDGKBMODE, &mut mode) };
            if rc < 0 {
                error!("KDGKBMODE ioctl failed: {}", io::Error::last_os_error());
                return;
            }

            if mode == K_OFF {
                info!("tty1 keyboard mode is K_OFF — VT input is disabled");
            } else {
                warn!(
                    "tty1 keyboard mode is active (mode={}) — VT may consume input",
                    mode
                );
            }
        }
    }
}

fn take_control() -> anyhow::Result<()> {
    check_vt_status();

    // Step 1: Create systemd-logind session
    info!("Start systemd-logind session with PAM");
    let session_id = start_pam_session()?;

    // Step 2: Connect to logind via D-Bus
    info!("Connect to logind via D-Bus");
    let conn = connect_to_dbus()?;

    // Step 3: Take control of the session (triggers VT muting)
    info!("Take control of the session (triggers VT muting)");
    send_take_control_message(&conn, &session_id)?;

    check_vt_status();

    // Step 4: Wait 120 seconds
    info!("Wait 120 seconds");
    thread::sleep(time::Duration::from_secs(120));

    // Step 5: Release control
    info!("Release control");
    send_release_control_message(&conn, &session_id)?;

    check_vt_status();

    Ok(())
}

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("debug")).init();

    info!("fallbackdm starting - minimalist systemd session controller");
    info!("Caution: This is a POC and automatically quits after 120 seconds");

    take_control()?;

    info!("fallbackdm shutdown complete");

    Ok(())
}
