// SPDX-License-Identifier: MIT
//
// Author: Drakulix (Victoria Brekenfeld)
// Author: Johannes Leupolz <dev@leupolz.eu>

// This is just the example of https://crates.io/crates/input with a tiny case distinction

use input::Event::Keyboard;
use input::{Libinput, LibinputInterface};
use std::fs::{File, OpenOptions};
use std::os::unix::{fs::OpenOptionsExt, io::OwnedFd};
use std::path::Path;

use libc::{O_RDONLY, O_RDWR, O_WRONLY};
use nix::poll::{self, PollFd, PollFlags, PollTimeout};
use std::os::fd::{AsRawFd, BorrowedFd};

pub struct Interface;

impl LibinputInterface for Interface {
    fn open_restricted(&mut self, path: &Path, flags: i32) -> Result<OwnedFd, i32> {
        OpenOptions::new()
            .custom_flags(flags)
            .read((flags & O_RDONLY != 0) | (flags & O_RDWR != 0))
            .write((flags & O_WRONLY != 0) | (flags & O_RDWR != 0))
            .open(path)
            .map(|file| file.into())
            .map_err(|err| err.raw_os_error().unwrap())
    }
    fn close_restricted(&mut self, fd: OwnedFd) {
        let _ = File::from(fd);
    }
}

pub fn wait_for_keyboard_event() {
    let mut input = Libinput::new_with_udev(Interface);
    input.udev_assign_seat("seat0").unwrap();
    let fd = unsafe { BorrowedFd::borrow_raw(input.as_raw_fd()) };
    let mut fds = [PollFd::new(fd, PollFlags::POLLIN)];

    loop {
        // Wait for events instead of busy-looping
        poll::poll(&mut fds, PollTimeout::NONE).unwrap();

        input.dispatch().unwrap();
        for event in &mut input {
            match &event {
                Keyboard(_keyboard_event) => {
                    println!("Got keyboard event: {:?}", event);
                    return;
                }
                _ => {
                    println!("Got irrelevant event: {:?}", event);
                }
            }
        }
    }
}
