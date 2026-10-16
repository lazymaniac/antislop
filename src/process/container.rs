//! An owned external container outlives its local CLI process. The guardian
//! must confirm its removal before releasing inherited workspace leases.
use crate::{Result, require};
use serde::{Deserialize, Serialize};
use std::{
    io::Read,
    path::PathBuf,
    process::Stdio,
    time::{Duration, Instant},
};

pub const OWNERSHIP_LABEL: &str = "io.antislop.check-invocation";
const INSPECT_BINDING: &str = r#"{"id":{{json .ID}},"image":{{json .Image}},"token":{{json (index .Config.Labels "io.antislop.check-invocation")}}}"#;
const INSPECT_CREATION: &str = r#"{"name":{{json .Name}},"id":{{json .ID}},"image":{{json .Image}},"token":{{json (index .Config.Labels "io.antislop.check-invocation")}}}"#;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    id: String,
    image: String,
    token: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NamedBinding {
    name: String,
    id: String,
    image: String,
    token: Option<String>,
}

/// Read-only recovery observation. Absence is deliberately not represented as
/// a proof: a previously submitted engine create may still finish later.
pub struct CreationObservation {
    name: String,
    resource: ContainerResource,
}
impl CreationObservation {
    pub(crate) fn name(&self) -> &str {
        &self.name
    }
    pub(crate) fn resource(&self) -> &ContainerResource {
        &self.resource
    }
}

pub fn observe_creation(
    binary: PathBuf,
    name: &str,
    image_id: &str,
    owner_token: &str,
) -> Result<Option<CreationObservation>> {
    require(
        name.strip_prefix("antislop-check-")
            .and_then(|id| uuid::Uuid::parse_str(id).ok())
            .is_some_and(|id| name == format!("antislop-check-{}", id.hyphenated())),
        "container creation observation requires its exact reserved name",
    )?;
    // The placeholder validates the engine/image/token only. Every operation
    // below selects the exact reserved name, then verifies the returned name.
    let mut resource = ContainerResource {
        binary,
        container_id: "0".repeat(64),
        image_id: image_id.into(),
        owner_token: owner_token.into(),
    };
    resource.validate()?;
    match resource.call(&["container", "exists", name])?.0 {
        Some(1) => return Ok(None),
        Some(0) => (),
        _ => {
            return Err(crate::Error::Invalid(
                "container creation observation is unavailable".into(),
            ));
        }
    }
    let (status, bytes) =
        resource.call(&["container", "inspect", "--format", INSPECT_CREATION, name])?;
    require(
        status == Some(0),
        "container creation identity is unconfirmed",
    )?;
    let observed: NamedBinding = crate::transport::decode(&bytes)?;
    require(
        observed.name == name
            && observed
                .image
                .strip_prefix("sha256:")
                .unwrap_or(&observed.image)
                == &image_id[7..]
            && observed.token.as_deref() == Some(owner_token),
        "container creation observation differs from its reserved ownership binding",
    )?;
    resource.container_id = observed.id;
    resource.validate()?;
    Ok(Some(CreationObservation {
        name: name.into(),
        resource,
    }))
}

/// Runtime-created ownership binding, never a model-selected cleanup command.
/// This is only lifecycle protection; it grants no check/session permission and
/// does not establish the container's filesystem/network restrictions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContainerResource {
    pub binary: PathBuf,
    pub container_id: String,
    pub image_id: String,
    pub owner_token: String,
}

/// Actual engine absence, not a deserializable/model-authored assertion.
/// Consuming this proof records cleanup only, never check execution or success.
pub struct RemovalProof {
    resource: ContainerResource,
}
impl RemovalProof {
    pub(crate) fn resource(&self) -> &ContainerResource {
        &self.resource
    }
}

fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

impl ContainerResource {
    pub fn validate(&self) -> Result<()> {
        require(
            self.binary.is_absolute()
                && self
                    .binary
                    .to_str()
                    .is_some_and(|p| !p.chars().any(char::is_control)),
            "container cleanup requires an absolute Unicode engine path",
        )?;
        require(
            digest(&self.container_id)
                && self.image_id.strip_prefix("sha256:").is_some_and(digest)
                && uuid::Uuid::parse_str(&self.owner_token)
                    .is_ok_and(|id| id.hyphenated().to_string() == self.owner_token),
            "container cleanup requires exact container/image IDs and an owned invocation token",
        )
    }

    fn call(&self, args: &[&str]) -> Result<(Option<i32>, Vec<u8>)> {
        // These are trusted engine-control operations, never repository argv.
        // The engine needs its normal connection configuration; none of these
        // variables is an instruction to pass the environment into a container.
        let mut command = super::command(&self.binary)?;
        command.env_clear();
        for name in [
            "HOME",
            "PATH",
            "TMPDIR",
            "XDG_RUNTIME_DIR",
            "CONTAINER_HOST",
            "CONTAINER_CONNECTION",
            "CONTAINER_SSHKEY",
        ] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = super::SupervisedChild::spawn(&mut command)?;
        let mut source = child.child.stdout.take().expect("engine stdout");
        let reader = std::thread::spawn(move || -> std::io::Result<(Vec<u8>, bool)> {
            let mut bytes = Vec::new();
            let mut buffer = [0; 8192];
            let mut truncated = false;
            loop {
                let count = source.read(&mut buffer)?;
                if count == 0 {
                    return Ok((bytes, truncated));
                }
                let keep = count.min((4096usize).saturating_sub(bytes.len()));
                bytes.extend_from_slice(&buffer[..keep]);
                truncated |= keep != count;
            }
        });
        let deadline = Instant::now() + Duration::from_secs(10);
        let status = loop {
            if let Some(status) = child.child.try_wait()? {
                break Some(status);
            }
            if Instant::now() >= deadline {
                child.cancel()?;
                break None;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        let (bytes, truncated) = reader
            .join()
            .map_err(|_| crate::Error::Invalid("engine output reader failed".into()))??;
        require(
            status.is_some() && !truncated,
            "container cleanup engine response is unavailable or exceeds its metadata bound",
        )?;
        Ok((status.and_then(|s| s.code()), bytes))
    }

    pub(crate) fn remove_once(&self) -> Result<bool> {
        // `exists` distinguishes absence (1) from unavailable storage/engine
        // (125). A failed inspect/removal alone is never proof of absence.
        match self.call(&["container", "exists", &self.container_id])?.0 {
            Some(1) => return Ok(true),
            Some(0) => (),
            _ => return Ok(false),
        }
        // Project only the three binding fields at the engine. Full inspect
        // would unnecessarily collect environment, mounts and user metadata.
        let (status, bytes) = self.call(&[
            "container",
            "inspect",
            "--format",
            INSPECT_BINDING,
            &self.container_id,
        ])?;
        if status != Some(0) {
            return Ok(false);
        }
        let record: Binding = crate::transport::decode(&bytes)?;
        let owned = record.id == self.container_id
            && record
                .image
                .strip_prefix("sha256:")
                .unwrap_or(&record.image)
                == &self.image_id[7..]
            && record.token.as_deref() == Some(self.owner_token.as_str());
        require(owned, "container cleanup ownership binding differs")?;
        let _ = self.call(&[
            "container",
            "rm",
            "--force",
            "--time",
            "0",
            &self.container_id,
        ])?;
        Ok(self.call(&["container", "exists", &self.container_id])?.0 == Some(1))
    }

    pub fn try_remove(&self) -> Result<Option<RemovalProof>> {
        self.validate()?;
        Ok(self.remove_once()?.then(|| RemovalProof {
            resource: self.clone(),
        }))
    }

    /// Only the guardian calls this after stopping its local child group. A
    /// disconnected engine or ownership mismatch cannot release the leases.
    pub(crate) fn finish(&self) {
        let mut announced = false;
        loop {
            if self.remove_once().is_ok_and(|removed| removed) {
                return;
            }
            if !announced {
                eprintln!(
                    "container cleanup is unconfirmed; guardian retains workspace leases until the owned resource is absent"
                );
                announced = true;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }
}
