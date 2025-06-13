// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use blsforme::bootloader::systemd_boot::{self};
use clap::{ArgMatches, Command};
use thiserror::Error;

use moss::Installation;

pub fn command() -> Command {
    Command::new("boot")
        .about("Boot management")
        .long_about("Manage boot configuration")
        .subcommand_required(true)
        .subcommand(Command::new("status").about("Status of boot configuration"))
}

/// Handle status for now
pub fn handle(_args: &ArgMatches, installation: Installation) -> Result<(), Error> {
    let root = installation.root.clone();
    let is_native = root.to_string_lossy() == "/";
    let config = blsforme::Configuration {
        root: if is_native {
            blsforme::Root::Native(root.clone())
        } else {
            blsforme::Root::Image(root.clone())
        },
        vfs: "/".into(),
    };

    let manager = blsforme::Manager::new(&config)?;
    match manager.boot_environment().firmware {
        blsforme::Firmware::Uefi => {
            println!("ESP            : {:?}", manager.boot_environment().esp());
            println!("XBOOTLDR       : {:?}", manager.boot_environment().xbootldr());
            if is_native {
                if let Ok(bootloader) = systemd_boot::interface::BootLoaderInterface::new(&config.vfs) {
                    let v = bootloader.get_ucs2_string(systemd_boot::interface::VariableName::Info)?;
                    println!("Bootloader     : {v}");
                }
            }
        }
        blsforme::Firmware::Bios => {
            println!("BOOT           : {:?}", manager.boot_environment().boot_partition());
        }
    }

    println!("Global cmdline : {:?}", manager.cmdline());

    Ok(())
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("blsforme")]
    Blsforme(#[from] blsforme::Error),

    #[error("sd_boot")]
    SdBoot(#[from] systemd_boot::interface::Error),

    #[error("io")]
    IO(#[from] std::io::Error),

    #[error("os-release")]
    OsRelease(#[from] blsforme::os_release::Error),
}
