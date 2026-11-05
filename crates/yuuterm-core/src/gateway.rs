//! SSH gateways (jump hosts) of a bookmark. Metadata only: secrets
//! and live transports stay in the desktop and engine crates.
use crate::{Profile, Protocol, SshAuth, SshSettings, valid_credential_id};
use serde::{Deserialize, Serialize};

pub const MAX_GATEWAYS: usize = 4;

/// One hop before the target, in connection order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum Gateway {
    /// A saved SSH bookmark, by stable ID. Only its host, port, user name,
    /// authentication and saved-password identity are used; never its own
    /// gateways.
    Saved {
        profile_id: u64,
    },
    Manual(ManualGateway),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ManualGateway {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth: SshAuth,
    pub key_file: String,
    pub agent_pipe: String,
    pub identity_fingerprint: String,
    /// Saved-password identity of this hop. Never copied or exported.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub credential_id: String,
}

impl Default for ManualGateway {
    fn default() -> Self {
        let ssh = SshSettings::default();
        Self {
            host: String::new(),
            port: 22,
            username: String::new(),
            auth: ssh.auth,
            key_file: String::new(),
            agent_pipe: ssh.agent_pipe,
            identity_fingerprint: String::new(),
            credential_id: String::new(),
        }
    }
}

impl ManualGateway {
    pub fn validate(&self) -> Result<(), &'static str> {
        crate::validate_host(&self.host)?;
        if self.port == 0 {
            return Err("Gateway port must be between 1 and 65535.");
        }
        crate::validate_ssh_identity(
            &self.username,
            self.auth,
            &self.agent_pipe,
            &[&self.key_file, &self.identity_fingerprint],
        )?;
        if !self.credential_id.is_empty() && !valid_credential_id(&self.credential_id) {
            return Err("Invalid saved-password identity.");
        }
        Ok(())
    }
}

/// A hop ready to connect: references replaced by the referenced bookmark.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedGateway {
    /// Position in the chain, 0-based.
    pub index: usize,
    /// The referenced bookmark, `None` for a manual hop.
    pub source_profile_id: Option<u64>,
    pub hop: ManualGateway,
}

/// Endpoint and identity of a saved SSH bookmark as a gateway hop.
pub fn gateway_from_profile(p: &Profile) -> ManualGateway {
    ManualGateway {
        host: p.host.clone(),
        port: p.port,
        username: p.username.clone(),
        auth: p.ssh.auth,
        key_file: p.ssh.key_file.clone(),
        agent_pipe: p.ssh.agent_pipe.clone(),
        identity_fingerprint: p.ssh.identity_fingerprint.clone(),
        credential_id: p.credential_id.clone(),
    }
}

/// Resolve against the current saved bookmarks; a referenced bookmark's own
/// gateways are never followed.
pub fn resolve_gateways(
    profile: &Profile,
    profiles: &[Profile],
) -> Result<Vec<ResolvedGateway>, String> {
    profile
        .gateways
        .iter()
        .enumerate()
        .map(|(index, gateway)| match gateway {
            Gateway::Manual(hop) => Ok(ResolvedGateway {
                index,
                source_profile_id: None,
                hop: hop.clone(),
            }),
            Gateway::Saved { profile_id } => {
                let source = profiles
                    .iter()
                    .find(|p| p.id == *profile_id && !p.temporary)
                    .ok_or_else(|| {
                        format!("Gateway {}: saved session no longer exists", index + 1)
                    })?;
                if source.protocol != Protocol::Ssh {
                    return Err(format!("Gateway {}: saved session is not SSH", index + 1));
                }
                Ok(ResolvedGateway {
                    index,
                    source_profile_id: Some(source.id),
                    hop: gateway_from_profile(source),
                })
            }
        })
        .collect()
}

/// Bookmarks that use `id` as a gateway.
pub fn gateway_dependents(profiles: &[Profile], id: u64) -> Vec<u64> {
    profiles
        .iter()
        .filter(|p| p.gateways.contains(&Gateway::Saved { profile_id: id }))
        .map(|p| p.id)
        .collect()
}

/// Before deleting `deleted`, turn references to them in the remaining
/// bookmarks into credential-free manual hops. Returns the number converted.
pub fn detach_gateway_references(profiles: &mut [Profile], deleted: &[u64]) -> usize {
    let sources: Vec<(u64, ManualGateway)> = profiles
        .iter()
        .filter(|p| deleted.contains(&p.id))
        .map(|p| {
            let mut hop = gateway_from_profile(p);
            hop.credential_id.clear();
            (p.id, hop)
        })
        .collect();
    let mut converted = 0;
    for profile in profiles.iter_mut().filter(|p| !deleted.contains(&p.id)) {
        for gateway in &mut profile.gateways {
            if let Gateway::Saved { profile_id } = gateway
                && let Some((_, hop)) = sources.iter().find(|(id, _)| id == profile_id)
            {
                *gateway = Gateway::Manual(hop.clone());
                converted += 1;
            }
        }
    }
    converted
}

/// Every reference names an existing, saved SSH bookmark.
pub(crate) fn validate_references(profiles: &[Profile]) -> Result<(), String> {
    for profile in profiles {
        resolve_gateways(profile, profiles).map_err(|e| format!("{}: {e}", profile.name))?;
    }
    Ok(())
}

/// Copies and exports never carry saved-password identities.
pub(crate) fn clear_gateway_credentials(profile: &mut Profile) {
    for gateway in &mut profile.gateways {
        if let Gateway::Manual(hop) = gateway {
            hop.credential_id.clear();
        }
    }
}

/// Count, protocol and self-reference rules. Whether a referenced bookmark
/// exists and is SSH is checked against the saved list when connecting.
pub(crate) fn validate_gateways(profile: &Profile) -> Result<(), &'static str> {
    if profile.gateways.is_empty() {
        return Ok(());
    }
    if !matches!(
        profile.protocol,
        Protocol::Ssh | Protocol::Telnet | Protocol::Rdp | Protocol::Vnc
    ) {
        return Err("Only SSH, TELNET, RDP and VNC sessions can use SSH gateways.");
    }
    if profile.gateways.len() > MAX_GATEWAYS {
        return Err("Use up to 4 SSH gateways.");
    }
    for gateway in &profile.gateways {
        match gateway {
            Gateway::Saved { profile_id } if *profile_id == profile.id => {
                return Err("A session cannot be its own SSH gateway.");
            }
            Gateway::Saved { .. } => {}
            Gateway::Manual(hop) => hop.validate()?,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Profile, Protocol};

    fn ssh(id: u64) -> Profile {
        Profile {
            id,
            name: "Target".into(),
            host: "target.example".into(),
            ..Default::default()
        }
    }

    fn manual(host: &str) -> Gateway {
        Gateway::Manual(ManualGateway {
            host: host.into(),
            username: "jump".into(),
            ..Default::default()
        })
    }

    #[test]
    fn old_profile_json_without_gateways_loads_empty() {
        let old: Profile = serde_json::from_str(
            r#"{"id":1,"name":"Old","protocol":"Ssh","host":"h","port":22,"username":"","font_size":null}"#,
        )
        .unwrap();
        assert!(old.gateways.is_empty());
        assert!(!serde_json::to_string(&old).unwrap().contains("gateways"));
    }

    #[test]
    fn more_than_four_gateways_rejected() {
        let mut p = ssh(1);
        p.gateways = (0..MAX_GATEWAYS).map(|_| manual("bastion")).collect();
        assert!(p.validate().is_ok());
        p.gateways.push(manual("bastion"));
        assert!(p.validate().is_err());
    }

    #[test]
    fn gateways_on_network_protocols_only() {
        for protocol in [
            Protocol::Ssh,
            Protocol::Telnet,
            Protocol::Rdp,
            Protocol::Vnc,
        ] {
            let mut p = ssh(1);
            p.protocol = protocol;
            p.port = protocol.default_port();
            p.gateways = vec![manual("bastion")];
            assert!(p.validate().is_ok(), "{protocol:?}");
        }
        let mut p = ssh(1);
        p.protocol = Protocol::Local;
        p.gateways = vec![manual("bastion")];
        assert!(p.validate().is_err());
    }

    #[test]
    fn self_reference_rejected() {
        let mut p = ssh(7);
        p.gateways = vec![Gateway::Saved { profile_id: 7 }];
        assert!(p.validate().is_err());
        p.gateways = vec![Gateway::Saved { profile_id: 8 }];
        assert!(p.validate().is_ok());
    }

    #[test]
    fn manual_rejects_option_like_host() {
        for host in ["-oProxyCommand=cmd", "host\ncommand", "", "host name"] {
            let mut p = ssh(1);
            p.gateways = vec![manual(host)];
            assert!(p.validate().is_err(), "{host:?}");
        }
    }

    #[test]
    fn manual_port_zero_rejected() {
        let mut p = ssh(1);
        p.gateways = vec![Gateway::Manual(ManualGateway {
            host: "bastion".into(),
            port: 0,
            ..Default::default()
        })];
        assert!(p.validate().is_err());
    }

    #[test]
    fn manual_password_requires_username_and_valid_identity() {
        let mut hop = ManualGateway {
            host: "bastion".into(),
            auth: crate::SshAuth::Password,
            ..Default::default()
        };
        assert!(hop.validate().is_err(), "password needs a user name");
        hop.username = "jump".into();
        assert!(hop.validate().is_ok());
        hop.credential_id = "not-a-uuid".into();
        assert!(hop.validate().is_err());
    }

    fn prefs(profiles: Vec<Profile>) -> crate::config::Preferences {
        let mut p = crate::config::Preferences::fresh();
        p.profiles = profiles;
        p
    }

    fn bastion(id: u64) -> Profile {
        Profile {
            id,
            name: "Bastion".into(),
            host: "bastion.example".into(),
            username: "admin".into(),
            ..Default::default()
        }
    }

    #[test]
    fn resolve_saved_uses_current_reference() {
        let mut target = ssh(1);
        target.gateways = vec![Gateway::Saved { profile_id: 2 }, manual("jump2")];
        let mut gw = bastion(2);
        gw.port = 2222;
        gw.credential_id = "a3bb189e-8bf9-3888-9912-ace4e6543002".into();
        let resolved = resolve_gateways(&target, &[target.clone(), gw.clone()]).unwrap();
        assert_eq!(resolved.len(), 2);
        assert_eq!(resolved[0].index, 0);
        assert_eq!(resolved[0].source_profile_id, Some(2));
        assert_eq!(resolved[0].hop.host, "bastion.example");
        assert_eq!(resolved[0].hop.port, 2222);
        assert_eq!(resolved[0].hop.credential_id, gw.credential_id);
        assert_eq!(resolved[1].source_profile_id, None);
        assert_eq!(resolved[1].hop.host, "jump2");
        gw.host = "moved.example".into();
        let again = resolve_gateways(&target, &[target.clone(), gw]).unwrap();
        assert_eq!(again[0].hop.host, "moved.example");
    }

    #[test]
    fn resolve_rejects_missing_reference() {
        let mut target = ssh(1);
        target.gateways = vec![manual("jump"), Gateway::Saved { profile_id: 9 }];
        assert_eq!(
            resolve_gateways(&target, &[target.clone()]).unwrap_err(),
            "Gateway 2: saved session no longer exists"
        );
    }

    #[test]
    fn resolve_rejects_non_ssh_reference() {
        let mut target = ssh(1);
        target.gateways = vec![Gateway::Saved { profile_id: 2 }];
        let mut gw = bastion(2);
        gw.protocol = Protocol::Telnet;
        assert_eq!(
            resolve_gateways(&target, &[target.clone(), gw]).unwrap_err(),
            "Gateway 1: saved session is not SSH"
        );
    }

    #[test]
    fn resolve_ignores_nested_gateways() {
        let mut target = ssh(1);
        target.gateways = vec![Gateway::Saved { profile_id: 2 }];
        let mut gw = bastion(2);
        gw.gateways = vec![manual("deeper")];
        let resolved = resolve_gateways(&target, &[target.clone(), gw]).unwrap();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].hop.host, "bastion.example");
    }

    #[test]
    fn preferences_reject_broken_references() {
        let mut target = ssh(1);
        target.gateways = vec![Gateway::Saved { profile_id: 2 }];
        assert!(prefs(vec![target.clone(), bastion(2)]).validate().is_ok());
        assert!(prefs(vec![target.clone()]).validate().is_err());
        let mut telnet = bastion(2);
        telnet.protocol = Protocol::Telnet;
        telnet.port = 23;
        assert!(prefs(vec![target, telnet]).validate().is_err());
    }

    #[test]
    fn dependents_and_detach_convert_references_of_deleted() {
        let mut a = ssh(1);
        a.gateways = vec![Gateway::Saved { profile_id: 3 }];
        let mut b = ssh(2);
        b.name = "B".into();
        b.gateways = vec![manual("x"), Gateway::Saved { profile_id: 3 }];
        let mut gw = bastion(3);
        gw.credential_id = "a3bb189e-8bf9-3888-9912-ace4e6543002".into();
        let mut profiles = vec![a, b, gw];
        assert_eq!(gateway_dependents(&profiles, 3), vec![1, 2]);
        assert_eq!(detach_gateway_references(&mut profiles, &[3]), 2);
        let Gateway::Manual(hop) = &profiles[1].gateways[1] else {
            panic!("reference not converted");
        };
        assert_eq!(hop.host, "bastion.example");
        assert_eq!(hop.username, "admin");
        assert!(hop.credential_id.is_empty());
        assert!(gateway_dependents(&profiles, 3).is_empty());
    }

    #[test]
    fn detach_skips_profiles_deleted_together() {
        let mut a = ssh(1);
        a.gateways = vec![Gateway::Saved { profile_id: 3 }];
        let mut profiles = vec![a, bastion(3)];
        assert_eq!(detach_gateway_references(&mut profiles, &[1, 3]), 0);
        assert_eq!(profiles[0].gateways, vec![Gateway::Saved { profile_id: 3 }]);
    }

    #[test]
    fn serde_roundtrip_tagged() {
        let mut p = ssh(1);
        p.gateways = vec![Gateway::Saved { profile_id: 2 }, manual("bastion")];
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains(r#""kind":"Saved""#), "{json}");
        assert!(json.contains(r#""kind":"Manual""#), "{json}");
        let back: Profile = serde_json::from_str(&json).unwrap();
        assert_eq!(back.gateways, p.gateways);
        let defaulted: ManualGateway =
            serde_json::from_str(r#"{"host":"b","username":"u"}"#).unwrap();
        assert_eq!(defaulted.port, 22);
    }
}
