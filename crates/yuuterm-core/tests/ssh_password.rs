use yuuterm_core::{Profile, SshAuth};
#[test]
fn password_mode_validates_without_identity_and_never_serializes_a_secret() {
    let mut p = Profile {
        name: "Synthetic password metadata".into(),
        host: "127.0.0.1".into(),
        username: "fixture".into(),
        ..Default::default()
    };
    p.ssh.auth = SshAuth::Password;
    p.ssh.agent_pipe.clear();
    assert!(p.validate().is_ok());
    let json = serde_json::to_string(&p).unwrap();
    assert!(json.contains("\"auth\":\"Password\"") && !json.contains("\"password\""));
    let decoded: Profile = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded.ssh.auth, SshAuth::Password);
    p.username.clear();
    assert!(p.validate().is_err());
}
