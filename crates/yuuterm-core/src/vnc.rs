//! VNC metadata contains no credentials or trust decisions.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum VncAuth {
    #[default]
    Plain,
    Vnc,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct VncSettings {
    pub auth: VncAuth,
    pub clipboard: bool,
    pub resize_server: bool,
}
