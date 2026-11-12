//! Real Windows ACL fixtures for the unsupported capability, NOT ACL enforcement.
//! Requires NTFS + Windows PowerShell; absence/failure is missing evidence, not a skip.
//! No production code invokes PowerShell, and no existing system ACL is modified.
#![cfg(windows)]
use kortex_repo::placement::{DirectoryAnchor, require_derived_data_support};
use std::{fs, path::PathBuf, process::Command};

#[test]
fn actual_permissive_inherited_and_foreign_owner_directories_fail_before_writes() {
    let d = tempfile::tempdir().unwrap();
    let system_root = PathBuf::from(std::env::var_os("SystemRoot").expect("Windows SystemRoot"));
    let powershell = system_root.join("System32/WindowsPowerShell/v1.0/powershell.exe");
    // These are synthetic, empty test objects. Everyone access is intentional to
    // exercise refusal; never place real derived/source data in this fixture.
    let script = r#"
$ErrorActionPreference = 'Stop'
$parent = $env:KORTEX_ACL_FIXTURE
$user = [System.Security.Principal.WindowsIdentity]::GetCurrent().User
$world = [System.Security.Principal.SecurityIdentifier]::new('S-1-1-0')
$inherit = [System.Security.AccessControl.InheritanceFlags]'ContainerInherit, ObjectInherit'
$propagation = [System.Security.AccessControl.PropagationFlags]::None
$allow = [System.Security.AccessControl.AccessControlType]::Allow
$acl = [System.Security.AccessControl.DirectorySecurity]::new()
$acl.SetOwner($user)
$acl.SetAccessRuleProtection($true, $false)
$acl.AddAccessRule([System.Security.AccessControl.FileSystemAccessRule]::new($user, 'FullControl', $inherit, $propagation, $allow))
$acl.AddAccessRule([System.Security.AccessControl.FileSystemAccessRule]::new($world, 'Modify', $inherit, $propagation, $allow))
Set-Acl -LiteralPath $parent -AclObject $acl
$inherited = [System.IO.Directory]::CreateDirectory((Join-Path $parent 'inherited')).FullName
$childAcl = Get-Acl -LiteralPath $inherited
$rules = @($childAcl.GetAccessRules($true, $true, [System.Security.Principal.SecurityIdentifier]) | Where-Object { $_.IsInherited -and $_.IdentityReference.Value -eq 'S-1-1-0' -and $_.AccessControlType -eq $allow })
if ($childAcl.AreAccessRulesProtected -or $rules.Count -eq 0) { throw 'NTFS Everyone inherited ACE fixture not established' }
$permissive = [System.IO.Directory]::CreateDirectory((Join-Path $parent 'permissive')).FullName
Set-Acl -LiteralPath $permissive -AclObject $acl
$explicit = Get-Acl -LiteralPath $permissive
$rules = @($explicit.GetAccessRules($true, $false, [System.Security.Principal.SecurityIdentifier]) | Where-Object { $_.IdentityReference.Value -eq 'S-1-1-0' -and $_.AccessControlType -eq $allow })
if (-not $explicit.AreAccessRulesProtected -or $rules.Count -eq 0) { throw 'Explicit permissive ACE fixture not established' }
$foreign = Get-Acl -LiteralPath $env:SystemRoot
if ($foreign.GetOwner([System.Security.Principal.SecurityIdentifier]).Value -eq $user.Value) { throw 'SystemRoot is not a foreign-owner fixture on this host' }
Write-Output 'REAL_ACL_FIXTURES_VERIFIED'
"#;
    let output = Command::new(powershell)
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .env("KORTEX_ACL_FIXTURE", d.path())
        .output()
        .expect("real Windows PowerShell ACL fixture setup must execute");
    assert!(
        output.status.success(),
        "ACL fixture setup failed (not accepted as platform evidence): {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("REAL_ACL_FIXTURES_VERIFIED"));
    assert_eq!(
        require_derived_data_support().unwrap_err().code,
        "PRIVATE_STORAGE_UNSUPPORTED"
    );
    for name in ["permissive", "inherited"] {
        let path = d.path().join(name);
        // An empty target is important: no derived bytes were written for testing.
        assert_eq!(fs::read_dir(&path).unwrap().count(), 0);
        for create in [false, true] {
            assert_eq!(
                DirectoryAnchor::acquire(&path, create).unwrap_err().code,
                "PRIVATE_STORAGE_UNSUPPORTED"
            );
        }
        assert_eq!(fs::read_dir(&path).unwrap().count(), 0);
        assert_eq!(
            DirectoryAnchor::acquire(&path.join("must-not-exist"), true)
                .unwrap_err()
                .code,
            "PRIVATE_STORAGE_UNSUPPORTED"
        );
        assert!(!path.join("must-not-exist").exists());
    }
    // Existing foreign-owner directory is read-only metadata fixture input.
    // acquire fails at the capability gate, not after trying to change its ACL.
    assert_eq!(
        DirectoryAnchor::acquire(&system_root, false)
            .unwrap_err()
            .code,
        "PRIVATE_STORAGE_UNSUPPORTED"
    );
}
