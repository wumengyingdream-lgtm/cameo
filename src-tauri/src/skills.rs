//! Bundled Codex skills — Cameo ships a set of skills, seeds their real bodies
//! into `~/.cameo/skills/` on boot, and symlinks the enabled ones into each Board
//! folder's `.agents/skills/` so the Codex sidecar discovers them as **repo-scope**
//! skills (the sidecar's cwd is the Board folder). That makes them ambiently
//! available to the agent and visible in the `/` skill menu, without ever writing
//! into the user's `~/.codex` (no `config.toml`, no global skill pollution).
//!
//! Mechanism + hardening are ported from MyAgents' validated `skill_sync.rs`:
//!
//! # Invariants
//!
//! 1. We NEVER write through a real (non-symlink) path. A user that hand-creates
//!    `<folder>/.agents/skills/foo/` keeps it — we don't shadow or delete it.
//! 2. We only DELETE entries that are links pointing INTO `~/.cameo/skills`.
//!    Anything else (a real dir, or a link the user made pointing elsewhere) is
//!    left alone.
//! 3. Existence is probed with `fs::symlink_metadata` (NOT `Path::exists` /
//!    `fs::metadata`) — a broken link must register as "occupied" so we re-point it
//!    instead of stepping past it into a confusing write.
//! 4. Dangling-link cleanup resolves targets with lexical `fs::read_link` (NOT
//!    `canonicalize`, which fails on broken links and would leak them forever).
//! 5. Windows directory links use NTFS **junctions** (`junction::create`) — they
//!    need no admin / Developer Mode, unlike true symlinks.

use include_dir::{include_dir, Dir};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Skills embedded in the binary at compile time (`src-tauri/skills/`). Seeded to
/// `~/.cameo/skills/` on boot. Bump [`BUNDLED_SKILLS_VERSION`] whenever the content
/// changes so existing installs re-seed.
static BUNDLED: Dir = include_dir!("$CARGO_MANIFEST_DIR/skills");

/// On-disk schema version of `registry.json` (bump → migration).
const REGISTRY_VERSION: u32 = 1;
/// Bump when the bundled skill content changes; triggers a re-seed on next boot.
const BUNDLED_SKILLS_VERSION: u32 = 1;

/// `~/.cameo/skills/registry.json` — tracks which bundled skills were seeded and
/// which are disabled. **Default-on**: a skill is enabled unless its name is in
/// `disabled`. There is no UI to edit `disabled` yet (it stays empty); the field +
/// `generation` are reserved so a future toggle UI needs no schema change.
#[derive(Serialize, Deserialize)]
struct Registry {
    #[serde(default)]
    version: u32,
    /// Last [`BUNDLED_SKILLS_VERSION`] written to disk.
    #[serde(default)]
    bundled_version: u32,
    /// Names of skills materialized by the last seed (used to prune stale dirs).
    #[serde(default)]
    seeded: Vec<String>,
    /// Skill names the user has turned off. Empty for now (no toggle UI).
    #[serde(default)]
    disabled: Vec<String>,
    /// Monotonic counter, reserved for lazy-sync de-dup once a toggle UI exists.
    #[serde(default)]
    generation: u64,
}

impl Default for Registry {
    fn default() -> Self {
        Registry {
            version: REGISTRY_VERSION,
            bundled_version: 0,
            seeded: Vec::new(),
            disabled: Vec::new(),
            generation: 0,
        }
    }
}

fn registry_path(root: &Path) -> PathBuf {
    root.join("registry.json")
}

fn read_registry(root: &Path) -> Registry {
    match fs::read_to_string(registry_path(root)) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => Registry::default(),
    }
}

fn write_registry(root: &Path, reg: &Registry) {
    if let Ok(s) = serde_json::to_string_pretty(reg) {
        let _ = fs::write(registry_path(root), s);
    }
}

/// Disabled-skill names from `registry.json` (missing file → none disabled).
fn read_disabled(root: &Path) -> HashSet<String> {
    read_registry(root).disabled.into_iter().collect()
}

/// Names of the top-level bundled skill directories.
fn bundled_names() -> Vec<String> {
    BUNDLED
        .dirs()
        .filter_map(|d| d.path().file_name().map(|s| s.to_string_lossy().to_string()))
        .collect()
}

// ── Seeding (boot) ───────────────────────────────────────────────────────────

/// Materialize the binary-embedded skills into `~/.cameo/skills/`. Idempotent and
/// cheap on the steady state (version match + dirs present → no-op). Called once at
/// boot, before any Board session can start. Best-effort: failures are logged, not
/// fatal (worst case the `/` menu is missing the bundled skills).
pub fn seed_bundled() {
    let root = crate::paths::cameo_skills_dir();
    if let Err(e) = fs::create_dir_all(&root) {
        tracing::warn!(module = "skills", "create {} failed: {e}", root.display());
        return;
    }
    let mut reg = read_registry(&root);
    let names = bundled_names();

    let up_to_date = reg.bundled_version == BUNDLED_SKILLS_VERSION
        && names
            .iter()
            .all(|n| root.join(n).join("SKILL.md").is_file());
    if up_to_date {
        return;
    }

    // Drop dirs we seeded in a prior version that are no longer bundled. Validate
    // each name is a plain path component first — a tampered registry entry like
    // "../x" must never turn this into a delete outside the skills store.
    for old in &reg.seeded {
        if is_plain_component(old) && !names.contains(old) {
            let _ = fs::remove_dir_all(root.join(old));
        }
    }
    // Only persist the new version if extraction fully succeeded; otherwise leave
    // the registry stale so the next boot retries (no false "up to date").
    if let Err(e) = extract_bundle_into(&root) {
        tracing::warn!(module = "skills", "seed extract failed, will retry next boot: {e}");
        return;
    }
    reg.version = REGISTRY_VERSION;
    reg.bundled_version = BUNDLED_SKILLS_VERSION;
    reg.seeded = names.clone();
    write_registry(&root, &reg);
    tracing::info!(module = "skills", count = names.len(), "seeded bundled skills");
}

fn extract_bundle_into(root: &Path) -> io::Result<()> {
    for d in BUNDLED.dirs() {
        // Fully reset the skill path before materializing it, so we never write
        // THROUGH a pre-existing link — at the top level OR nested inside (a real
        // dir containing a symlinked `SKILL.md` would otherwise be followed by
        // `fs::write`). The whole `~/.cameo/skills/<skill>` subtree is Cameo-owned:
        // a squatting link is dropped by its link (target untouched); our own prior
        // real dir is wiped (`remove_dir_all` unlinks nested symlinks WITHOUT
        // following them).
        if let Some(name) = d.path().file_name().and_then(|s| s.to_str()) {
            if !is_plain_component(name) {
                continue;
            }
            let top = root.join(name);
            match fs::symlink_metadata(&top) {
                Ok(m) if is_managed_link(&m) => {
                    let _ = remove_link(&top);
                }
                Ok(_) => {
                    let _ = fs::remove_dir_all(&top);
                }
                Err(_) => {}
            }
        }
        extract_dir(d, root)?;
    }
    Ok(())
}

fn extract_dir(dir: &Dir, root: &Path) -> io::Result<()> {
    fs::create_dir_all(root.join(dir.path()))?;
    for f in dir.files() {
        let p = root.join(f.path());
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&p, f.contents())?;
    }
    for sub in dir.dirs() {
        extract_dir(sub, root)?;
    }
    Ok(())
}

// ── Workspace injection (per Board session start) ────────────────────────────

/// Link the enabled bundled skills into `<folder>/.agents/skills/`. Idempotent;
/// safe to call on every session start. MUST run before the Codex sidecar spawns
/// in `folder` so the app-server scans the links as repo-scope skills. Best-effort:
/// a failure leaves the agent with fewer skills, never blocks the session.
pub fn ensure_workspace_skills(folder: &Path) {
    let src = crate::paths::cameo_skills_dir();
    if !src.is_dir() {
        return;
    }
    let dst = folder.join(".agents").join("skills");
    if let Err(e) = fs::create_dir_all(&dst) {
        tracing::warn!(
            module = "skills",
            folder = %folder.display(),
            "create .agents/skills failed: {e}"
        );
        return;
    }
    sync_into(&src, &dst, &read_disabled(&src));
    git_exclude(folder);
}

/// Pure inject+cleanup over explicit dirs (the testable core). For each enabled
/// skill in `src` (a dir with a `SKILL.md`), ensure `dst/<name>` links to it; then
/// remove any of OUR stale links in `dst` (disabled / deleted / renamed).
fn sync_into(src: &Path, dst: &Path, disabled: &HashSet<String>) {
    let mut enabled: HashSet<String> = HashSet::new();
    let entries = match fs::read_dir(src) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let target = entry.path();
        // A skill is a directory containing SKILL.md — skips registry.json & strays.
        if !target.join("SKILL.md").is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if disabled.contains(&name) {
            continue; // not enabled → don't link; cleanup_dangling drops any old link
        }
        let link = dst.join(&name);
        enabled.insert(name);
        match fs::symlink_metadata(&link) {
            // Real, user-authored dir of the same name — never shadow or remove it.
            Ok(m) if !is_managed_link(&m) => continue,
            // An existing link. Keep it if already correct. Re-point only if it's
            // OURS (points into src); a user's own link with a colliding name is
            // left untouched (same courtesy as a real dir).
            Ok(_) => match fs::read_link(&link) {
                Ok(t) if t == target => continue,
                Ok(t) if link_points_into(&t, dst, src) => {
                    let _ = remove_link(&link);
                }
                _ => continue,
            },
            // Nothing there → create below.
            Err(_) => {}
        }
        if let Err(e) = create_link_dir(&target, &link) {
            if e.kind() == io::ErrorKind::AlreadyExists {
                tracing::debug!(
                    module = "skills",
                    "skill link {} already exists (concurrent sync) — ignoring",
                    link.display()
                );
            } else {
                tracing::warn!(
                    module = "skills",
                    "skill link {} -> {} failed: {e}",
                    link.display(),
                    target.display()
                );
            }
        }
    }
    cleanup_dangling(dst, src, &enabled);
}

/// Remove links in `dst` that we manage but that are no longer wanted: their name
/// isn't in `keep` (disabled / deleted / renamed source) AND they point into `src`.
/// Real dirs and links pointing elsewhere are never touched.
fn cleanup_dangling(dst: &Path, src: &Path, keep: &HashSet<String>) {
    let entries = match fs::read_dir(dst) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if keep.contains(&name) {
            continue;
        }
        let link = entry.path();
        let meta = match fs::symlink_metadata(&link) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if !is_managed_link(&meta) {
            continue; // real user dir/file — leave alone
        }
        // Lexical resolution: read the link target without traversing it, so broken
        // links still resolve to a path we can prefix-check.
        let target = match fs::read_link(&link) {
            Ok(t) => t,
            Err(_) => continue,
        };
        if !link_points_into(&target, dst, src) {
            continue; // resolves (lexically) outside our store — a link the user owns
        }
        let _ = remove_link(&link);
    }
}

/// Add `/.agents/` to the repo's `.git/info/exclude` (NOT the user's `.gitignore`)
/// so the injected links don't show up in `git status`. Idempotent; no-op when the
/// folder isn't a git repo.
fn git_exclude(folder: &Path) {
    let git_dir = folder.join(".git");
    if !git_dir.is_dir() {
        return; // not a repo, or a worktree/submodule .git file — skip
    }
    let info = git_dir.join("info");
    let exclude = info.join("exclude");
    // Don't follow a symlinked exclude file — it could point outside the repo and
    // we'd overwrite the target.
    if let Ok(m) = fs::symlink_metadata(&exclude) {
        if m.file_type().is_symlink() {
            return;
        }
    }
    let needle = "/.agents/";
    let existing = fs::read_to_string(&exclude).unwrap_or_default();
    if existing.lines().any(|l| l.trim() == needle) {
        return;
    }
    if fs::create_dir_all(&info).is_err() {
        return;
    }
    let mut content = existing;
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str("# Cameo: injected skill symlinks\n");
    content.push_str(needle);
    content.push('\n');
    let _ = fs::write(&exclude, content);
}

// ── Path safety ──────────────────────────────────────────────────────────────

/// A single safe path component — no separators, not `.`/`..`, not empty, not a
/// root/prefix. Guards names that come from `registry.json` (possibly tampered)
/// before they index into the filesystem for deletion.
fn is_plain_component(name: &str) -> bool {
    let mut comps = Path::new(name).components();
    matches!(comps.next(), Some(std::path::Component::Normal(_))) && comps.next().is_none()
}

/// Lexically collapse `.`/`..` without touching disk (an ownership decision must be
/// strict/textual: if a target *textually* escapes, we refuse to treat it as ours).
fn lexical_normalize(p: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Does `target` (a link's stored target, possibly relative to `base`) resolve —
/// lexically, rejecting `..` escapes — to a path inside `src`? This is the ownership
/// test: only links pointing INTO `~/.cameo/skills` are ours to remove/re-point.
fn link_points_into(target: &Path, base: &Path, src: &Path) -> bool {
    let abs = if target.is_absolute() {
        target.to_path_buf()
    } else {
        base.join(target)
    };
    lexical_normalize(&abs).starts_with(lexical_normalize(src))
}

// ── Platform link primitives ─────────────────────────────────────────────────

#[cfg(unix)]
fn create_link_dir(target: &Path, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn create_link_dir(target: &Path, link: &Path) -> io::Result<()> {
    // NTFS junction — works without admin / Developer Mode (true symlinks don't).
    junction::create(target, link)
}

/// Is this metadata one of OUR links (a symlink, or — on Windows — a directory
/// junction)? Rust's `is_symlink()` does not flag junctions, so check the reparse
/// attribute explicitly.
#[cfg(unix)]
fn is_managed_link(meta: &fs::Metadata) -> bool {
    meta.file_type().is_symlink()
}

#[cfg(windows)]
fn is_managed_link(meta: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    meta.file_type().is_symlink()
        || (meta.is_dir() && (meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0)
}

/// Remove a link without touching its target's contents. A directory link (Windows
/// junction) needs `remove_dir`; a file/unix symlink needs `remove_file`. We only
/// call this on paths confirmed to be managed links.
fn remove_link(link: &Path) -> io::Result<()> {
    let m = fs::symlink_metadata(link)?;
    // Re-verify it's still a link at delete time (TOCTOU): if the path was swapped
    // for a real directory between validation and here, refuse — never `remove_dir`
    // a real user dir.
    //
    // Residual (accepted): a path-based API can't fully close the window. On Windows
    // a junction confirmed here could be swapped for a real EMPTY dir before
    // `remove_dir` deletes it. Impact is one empty directory, and the attacker
    // already needs write access to the target dir; fully closing it needs
    // handle-based reparse deletion (not in std) — not worth a winapi dep.
    if !is_managed_link(&m) {
        return Err(io::Error::other("refusing to remove a non-link path"));
    }
    // `symlink_metadata` does not follow links, so `is_dir()` is true only for a
    // directory reparse point (junction), not a unix symlink-to-dir.
    if m.is_dir() {
        fs::remove_dir(link)
    } else {
        fs::remove_file(link)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn mk_skill(src: &Path, name: &str) {
        let d = src.join(name);
        fs::create_dir_all(&d).unwrap();
        fs::write(
            d.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: test\n---\n"),
        )
        .unwrap();
    }

    #[test]
    fn extract_writes_bundled_skill() {
        let root = tempdir().unwrap();
        extract_bundle_into(root.path()).unwrap();
        // The shipped video-edit skill must land with its SKILL.md.
        assert!(root.path().join("video-edit").join("SKILL.md").is_file());
    }

    #[test]
    fn injects_enabled_skips_disabled() {
        let src = tempdir().unwrap();
        let dst = tempdir().unwrap();
        mk_skill(src.path(), "a");
        mk_skill(src.path(), "b");
        let disabled: HashSet<String> = ["b".to_string()].into_iter().collect();
        sync_into(src.path(), dst.path(), &disabled);
        // 'a' is linked and resolves through to the source SKILL.md.
        assert!(dst.path().join("a").join("SKILL.md").is_file());
        // 'b' is disabled → no link.
        assert!(fs::symlink_metadata(dst.path().join("b")).is_err());
    }

    #[test]
    fn idempotent_keeps_correct_link() {
        let src = tempdir().unwrap();
        let dst = tempdir().unwrap();
        mk_skill(src.path(), "a");
        let none = HashSet::new();
        sync_into(src.path(), dst.path(), &none);
        sync_into(src.path(), dst.path(), &none); // second pass must not break it
        assert!(dst.path().join("a").join("SKILL.md").is_file());
    }

    #[test]
    fn preserves_user_real_dir() {
        let src = tempdir().unwrap();
        let dst = tempdir().unwrap();
        mk_skill(src.path(), "a");
        // User hand-authored a real dir with the same name as a bundled skill.
        let real = dst.path().join("a");
        fs::create_dir_all(&real).unwrap();
        fs::write(real.join("SKILL.md"), "USER").unwrap();
        sync_into(src.path(), dst.path(), &HashSet::new());
        let m = fs::symlink_metadata(&real).unwrap();
        assert!(!is_managed_link(&m), "user dir must not become a link");
        assert_eq!(fs::read_to_string(real.join("SKILL.md")).unwrap(), "USER");
    }

    #[test]
    fn cleanup_removes_stale_managed_link() {
        let src = tempdir().unwrap();
        let dst = tempdir().unwrap();
        mk_skill(src.path(), "a");
        sync_into(src.path(), dst.path(), &HashSet::new());
        assert!(fs::symlink_metadata(dst.path().join("a")).is_ok());
        // Source skill removed → its link must be cleaned on the next sync.
        fs::remove_dir_all(src.path().join("a")).unwrap();
        sync_into(src.path(), dst.path(), &HashSet::new());
        assert!(fs::symlink_metadata(dst.path().join("a")).is_err());
    }

    #[test]
    fn cleanup_leaves_foreign_link() {
        let src = tempdir().unwrap();
        let dst = tempdir().unwrap();
        let other = tempdir().unwrap();
        mk_skill(other.path(), "x");
        fs::create_dir_all(dst.path()).unwrap();
        // A managed-style link pointing OUTSIDE our store — user's own, leave it.
        create_link_dir(&other.path().join("x"), &dst.path().join("x")).unwrap();
        sync_into(src.path(), dst.path(), &HashSet::new());
        assert!(fs::symlink_metadata(dst.path().join("x")).is_ok());
    }

    #[test]
    fn git_exclude_is_idempotent() {
        let folder = tempdir().unwrap();
        fs::create_dir_all(folder.path().join(".git")).unwrap();
        git_exclude(folder.path());
        git_exclude(folder.path());
        let body = fs::read_to_string(folder.path().join(".git").join("info").join("exclude")).unwrap();
        assert_eq!(body.matches("/.agents/").count(), 1);
    }

    #[test]
    fn foreign_link_with_enabled_name_preserved() {
        // A user's own link at a name that collides with a bundled skill must NOT be
        // deleted or re-pointed — same courtesy as a real dir.
        let src = tempdir().unwrap();
        let dst = tempdir().unwrap();
        let other = tempdir().unwrap();
        mk_skill(src.path(), "video-edit");
        mk_skill(other.path(), "video-edit");
        fs::create_dir_all(dst.path()).unwrap();
        create_link_dir(&other.path().join("video-edit"), &dst.path().join("video-edit")).unwrap();
        sync_into(src.path(), dst.path(), &HashSet::new());
        assert_eq!(
            fs::read_link(dst.path().join("video-edit")).unwrap(),
            other.path().join("video-edit"),
            "foreign link must be left pointing at the user's dir"
        );
    }

    #[test]
    fn cleanup_rejects_dotdot_escape() {
        // A link whose target textually starts with src but escapes via `..` must be
        // treated as foreign (not deleted).
        let src = tempdir().unwrap();
        let dst = tempdir().unwrap();
        let foreign = src.path().parent().unwrap().join("foreign-skill");
        fs::create_dir_all(&foreign).unwrap();
        fs::create_dir_all(dst.path()).unwrap();
        let escaping = src.path().join("..").join("foreign-skill");
        create_link_dir(&escaping, &dst.path().join("z")).unwrap();
        sync_into(src.path(), dst.path(), &HashSet::new()); // src has no skills
        assert!(
            fs::symlink_metadata(dst.path().join("z")).is_ok(),
            "a `..`-escaping link must survive cleanup"
        );
    }

    #[test]
    fn is_plain_component_rejects_traversal() {
        assert!(is_plain_component("video-edit"));
        assert!(!is_plain_component(".."));
        assert!(!is_plain_component("."));
        assert!(!is_plain_component(""));
        assert!(!is_plain_component("a/b"));
        assert!(!is_plain_component("/abs"));
    }

    #[test]
    fn remove_link_refuses_real_dir() {
        let d = tempdir().unwrap();
        let real = d.path().join("real");
        fs::create_dir_all(&real).unwrap();
        assert!(remove_link(&real).is_err(), "must refuse to remove a real dir");
        assert!(real.is_dir(), "real dir must survive a refused remove_link");
    }

    // Seeding must reset a pre-existing skill subtree (here: a real dir whose
    // SKILL.md is a symlink pointing outside) instead of writing through it.
    #[cfg(unix)]
    #[test]
    fn seed_does_not_write_through_nested_symlink() {
        let root = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let secret = outside.path().join("secret.txt");
        fs::write(&secret, "DO NOT TOUCH").unwrap();
        let vid = root.path().join("video-edit");
        fs::create_dir_all(&vid).unwrap();
        std::os::unix::fs::symlink(&secret, vid.join("SKILL.md")).unwrap();
        extract_bundle_into(root.path()).unwrap();
        assert_eq!(
            fs::read_to_string(&secret).unwrap(),
            "DO NOT TOUCH",
            "must not write through a nested symlink to an outside file"
        );
        let m = fs::symlink_metadata(vid.join("SKILL.md")).unwrap();
        assert!(!m.file_type().is_symlink(), "SKILL.md must be a real file now");
        assert!(
            fs::read_to_string(vid.join("SKILL.md")).unwrap().contains("video-edit"),
            "real bundled SKILL.md must be materialized"
        );
    }
}
