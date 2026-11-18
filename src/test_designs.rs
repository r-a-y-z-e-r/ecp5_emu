use std::path::PathBuf;

#[path = "../tests/common/reference.rs"]
mod test_reference;

pub(crate) fn reference_design(name: &str) -> PathBuf {
    if name != "life_full" {
        return test_reference::build(name);
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/reference/life_full");
    assert!(
        ["top.config", "top_routed.json"]
            .iter()
            .all(|name| root.join(name).is_file()),
        "target/reference/life_full is not built (about 40 minutes of place and route): examples/ulx3s/build.sh examples/ulx3s/life_full target/reference/life_full"
    );
    root
}
