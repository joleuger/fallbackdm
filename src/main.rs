// SPDX-License-Identifier: MIT
//
// Author: Johannes Leupolz <dev@leupolz.eu>

// apt install libdbus-1-dev pkg-config libpam0g-dev

use std::time;
use std::time::Duration;

use log::info;
 use log::debug;
 use dbus::blocking::Connection;

 use std::thread;

use pam::Client;

fn start_pam_session() -> anyhow::Result<String> {
    let mut client = Client::with_password("fallbackdm")
        .expect("Failed to init PAM client.");
    client.conversation_mut().set_credentials("root", "irrelevant");

    // Actually try to authenticate:
    client.authenticate().expect("Authentication failed!");
    // Now that we are authenticated, it's possible to open a sesssion:
    client.open_session().expect("Failed to open a session!");


    // Get the current process's session ID from systemd-logind
    // read from /proc/self/sessionid
    let session_id=std::fs::read_to_string("/proc/self/sessionid")?;

    Ok(session_id)
}


fn connect_to_dbus() -> anyhow::Result<Connection> {
    let conn = Connection::new_system().expect("failed to connect to system bus");

    Ok(conn)
}

fn send_take_control_message(conn:&Connection, session:&str) -> anyhow::Result<()> {
    // https://www.freedesktop.org/software/systemd/man/latest/org.freedesktop.login1.html

    // can be found via
    // busctl --system list
    // busctl introspect --system org.freedesktop.Notifications
    let node = format!("/org/freedesktop/login1/session/{}",session);

    // create a wrapper struct around the connection
    let proxy = conn.with_proxy("org.freedesktop.login1", &node, Duration::from_millis(5000));

    // introspect for debugging
    let (xml,): (String,) = proxy.method_call(
        "org.freedesktop.DBus.Introspectable",
        "Introspect",
        (),
    )?;

    debug!("introspect dbus node {}: {}", &node, xml);

    // Now make the method call. The ListNames method call takes zero input parameters and
    // one output parameter which is an array of strings.
    // Therefore the input is a zero tuple "()", and the output is a single tuple "(names,)".
    let (): () = proxy.method_call("org.freedesktop.login1.Session", "TakeControl", (false,))?;

    Ok(())
}


fn send_release_control_message(conn:&Connection, session:&str) -> anyhow::Result<()> {
    // https://www.freedesktop.org/software/systemd/man/latest/org.freedesktop.login1.html

    // can be found via
    // busctl --system list
    // busctl introspect --system org.freedesktop.Notifications
    let node = format!("/org/freedesktop/login1/session/{}",session);

    // create a wrapper struct around the connection
    let proxy = conn.with_proxy("org.freedesktop.login1", &node, Duration::from_millis(5000));

    // Now make the method call. The ListNames method call takes zero input parameters and
    // one output parameter which is an array of strings.
    // Therefore the input is a zero tuple "()", and the output is a single tuple "(names,)".
    let (): () = proxy.method_call("org.freedesktop.login1.Session", "ReleaseControl", ())?;

    Ok(())
}

fn take_control() -> anyhow::Result<()> {

    // Step 1: Create systemd-logind session
    info!("Start systemd-logind session with PAM");
    let session_id = start_pam_session()?;

    // Step 2: Connect to logind via D-Bus
    info!("Connect to logind via D-Bus");
    let conn = connect_to_dbus()?;

    // Step 3: Take control of the session (triggers VT muting)
    info!("Take control of the session (triggers VT muting)");
    send_take_control_message(&conn,&session_id)?;

    // Step 4: Wait 120 seconds
    info!("Wait 120 seconds");
    thread::sleep(time::Duration::from_secs(20));

    // Step 5: Release control
    info!("Release control");
    send_release_control_message(&conn,&session_id)?;

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
