#![cfg_attr(not(feature = "host"), no_std)]

extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};

use libami::{AmiClient, AmiError, AmiValue};

pub struct ServiceLifecycle {
    name: String,
    ami: AmiClient,
}

impl ServiceLifecycle {
    pub fn connect(name: &str) -> Result<Self, AmiError> {
        Ok(Self {
            name: name.to_string(),
            ami: AmiClient::connect(name)?,
        })
    }

    pub fn service_name(&self) -> &str {
        &self.name
    }

    pub fn notify_starting(&mut self) -> Result<(), AmiError> {
        let started_at = current_uptime_ms() as i64;
        let tid = current_tid() as i64;
        let _ = self.ami.del(&self.key("error"));
        let _ = self.ami.del(&self.key("stopped_at"));
        let _ = self.ami.del(&self.key("failed_at"));
        self.ami.set(
            &self.key("state"),
            AmiValue::String(String::from("starting")),
        )?;
        self.ami.set(&self.key("ready"), AmiValue::Bool(false))?;
        self.ami.set(&self.key("tid"), AmiValue::Int(tid))?;
        self.ami
            .set(&self.key("started_at"), AmiValue::Int(started_at))?;
        Ok(())
    }

    pub fn notify_ready(&mut self) -> Result<(), AmiError> {
        self.ami.set(&self.key("ready"), AmiValue::Bool(true))?;
        self.ami
            .set(&self.key("state"), AmiValue::String(String::from("ready")))?;
        let _ = self.ami.del(&self.key("error"));
        Ok(())
    }

    pub fn notify_failed(&mut self, reason: &str) -> Result<(), AmiError> {
        let failed_at = current_uptime_ms() as i64;
        self.ami.set(&self.key("ready"), AmiValue::Bool(false))?;
        self.ami
            .set(&self.key("state"), AmiValue::String(String::from("failed")))?;
        self.ami
            .set(&self.key("error"), AmiValue::String(String::from(reason)))?;
        self.ami
            .set(&self.key("failed_at"), AmiValue::Int(failed_at))?;
        Ok(())
    }

    pub fn notify_stopping(&mut self) -> Result<(), AmiError> {
        let stopped_at = current_uptime_ms() as i64;
        self.ami.set(&self.key("ready"), AmiValue::Bool(false))?;
        self.ami.set(
            &self.key("state"),
            AmiValue::String(String::from("stopping")),
        )?;
        self.ami
            .set(&self.key("stopped_at"), AmiValue::Int(stopped_at))?;
        Ok(())
    }

    pub fn set_health(&mut self, health: &str) -> Result<(), AmiError> {
        self.ami
            .set(&self.key("health"), AmiValue::String(String::from(health)))?;
        Ok(())
    }

    pub fn set_detail(&mut self, key: &str, value: &str) -> Result<(), AmiError> {
        self.ami
            .set(&self.key(key), AmiValue::String(String::from(value)))?;
        Ok(())
    }

    fn key(&self, suffix: &str) -> String {
        format!("svc.{}.{}", self.name, suffix)
    }
}

fn current_tid() -> u32 {
    #[cfg(feature = "host")]
    {
        1
    }

    #[cfg(not(feature = "host"))]
    {
        libsyscall::get_tid()
    }
}

fn current_uptime_ms() -> u32 {
    #[cfg(feature = "host")]
    {
        0
    }

    #[cfg(not(feature = "host"))]
    {
        libsyscall::uptime_ms()
    }
}
