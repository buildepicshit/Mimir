//! Workspace write-lock integration tests.

use mimir_core::WorkspaceWriteLock;

#[test]
fn write_lock_excludes_second_holder_until_drop() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let log_path = tmp.path().join("canonical.log");

    let first = WorkspaceWriteLock::acquire_for_log(&log_path)?;
    let second = WorkspaceWriteLock::acquire_for_log(&log_path);
    assert!(second.is_err(), "second holder must not acquire lock");

    drop(first);
    let _second = WorkspaceWriteLock::acquire_for_log(&log_path)?;
    Ok(())
}

#[test]
#[cfg(unix)]
fn drop_does_not_remove_replaced_lockfile() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let log_path = tmp.path().join("canonical.log");

    let first = WorkspaceWriteLock::acquire_for_log(&log_path)?;
    let lock_path = first.path().to_path_buf();
    std::fs::remove_file(&lock_path)?;
    let second = WorkspaceWriteLock::acquire_for_log(&log_path)?;

    drop(first);
    assert!(
        lock_path.exists(),
        "first holder must not remove a replacement lockfile"
    );

    drop(second);
    assert!(
        !lock_path.exists(),
        "second holder should clean up its lock"
    );
    Ok(())
}
