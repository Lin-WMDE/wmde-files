// SPDX-License-Identifier: GPL-3.0-only
// Implementation of https://www.freedesktop.org/wiki/Specifications/file-manager-interface/

#![allow(dead_code, non_snake_case)]

use std::process;

pub struct FileManager;

impl FileManager {
    //TODO: return error?
    fn open(&self, uris: &[&str], _startup_id: &str) {
        match process::Command::new("wmde-files").args(uris).spawn() {
            Ok(mut child) => {
                log::info!("spawned wmde-files with id {:?}", child.id());
                match child.wait() {
                    Ok(status) => {
                        if status.success() {
                            log::info!("wmde-files exited with {status}");
                        } else {
                            log::warn!("failed to run wmde-files: exited with {status}");
                        }
                    }
                    Err(err) => {
                        log::warn!("failed to run wmde-files: {err}");
                    }
                }
            }
            Err(err) => {
                log::warn!("failed to spawn wmde-files: {err}");
            }
        }
    }
}

//TODO: why does &[&str] not implement Deserialize?
#[zbus::interface(name = "org.freedesktop.FileManager1")]
impl FileManager {
    fn ShowFolders(&self, URIs: Vec<&str>, StartupId: &str) {
        log::warn!("ShowFolders {:?} {:?}", URIs, StartupId);
        self.open(&URIs, StartupId)
    }

    fn ShowItems(&self, URIs: Vec<&str>, StartupId: &str) {
        log::warn!("ShowItems {:?} {:?}", URIs, StartupId);
        self.open(&URIs, StartupId)
    }

    fn ShowItemProperties(&self, URIs: Vec<&str>, StartupId: &str) {
        log::warn!("ShowItemProperties {:?} {:?}", URIs, StartupId);
        self.open(&URIs, StartupId)
    }
}
