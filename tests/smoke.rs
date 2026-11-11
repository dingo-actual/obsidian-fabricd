//! Integration smoke test: the crate builds as a library and advertises wire
//! protocol version `1`.

#[test]
fn protocol_version_is_one() {
    assert_eq!(obsidian_fabricd::protocol::PROTOCOL_VERSION, 1);
}
