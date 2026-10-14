//! Test-only data, intended for a private dbus-run-session and temporary HOME.
use secret_service::{blocking::SecretService, EncryptionType};
use std::collections::HashMap;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var("AQUA_TEST_KEYRING").as_deref() != Ok("1") {
        return Err("refusing to seed a non-test password store".into());
    }
    let service = SecretService::connect(EncryptionType::Dh)?;
    let collection = service.get_default_collection()?;
    collection.create_item(
        "Demo account",
        HashMap::from([("xdg:schema", "org.aqua.Password"), ("aqua-app-id", "org.aqua.InputTest")]),
        b"demo-secret",
        true,
        "text/plain",
    )?;
    collection.create_item(
        "Other app — must not be listed",
        HashMap::from([("xdg:schema", "org.aqua.Password"), ("aqua-app-id", "org.aqua.OtherTest")]),
        b"other-demo",
        true,
        "text/plain",
    )?;
    let entries = aqua_text::passwords::list("org.aqua.InputTest")?;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].label, "Demo account");
    let value = aqua_text::passwords::fetch("org.aqua.InputTest", &entries[0].path)?.unwrap();
    assert_eq!(&*value, "demo-secret");
    assert!(aqua_text::passwords::fetch("org.aqua.OtherTest", &entries[0].path)?.is_none());
    println!("FIXTURE_READY scoped Secret Service tests passed");
    Ok(())
}
