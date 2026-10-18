use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

const CACHEDIR_TAG: &str = "CACHEDIR.TAG";
const CACHEDIR_SIGNATURE: &[u8] = b"Signature: 8a477f597d28d172789f06886806bc55";

// Ported from tw93/Mole cmd/analyze/cleanable.go:isCleanableDir (GPL-3.0)
pub fn is_cleanable_dir(path: &Path) -> bool {
    let Some(name) = path.file_name() else {
        return false;
    };
    if has_valid_cache_dir_tag(path) {
        return true;
    }
    ARTIFACT_DIRS.contains(&name.to_string_lossy().as_ref())
}

// Ported from tw93/Mole cmd/analyze/cleanable.go:hasValidCacheDirTag (GPL-3.0)
fn has_valid_cache_dir_tag(path: &Path) -> bool {
    let tag_path = path.join(CACHEDIR_TAG);
    let Ok(metadata) = fs::symlink_metadata(&tag_path) else {
        return false;
    };
    if !metadata.file_type().is_file() {
        return false;
    }
    let Ok(mut file) = File::open(tag_path) else {
        return false;
    };
    let mut signature = [0_u8; CACHEDIR_SIGNATURE.len()];
    if file.read_exact(&mut signature).is_err() {
        return false;
    }
    signature == CACHEDIR_SIGNATURE
}

// Ported from tw93/Mole cmd/analyze/cleanable.go:projectDependencyDirs (GPL-3.0)
const ARTIFACT_DIRS: &[&str] = &[
    "node_modules",
    "bower_components",
    ".yarn",
    ".pnpm-store",
    "venv",
    ".venv",
    "virtualenv",
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    ".tox",
    ".eggs",
    "htmlcov",
    ".ipynb_checkpoints",
    "vendor",
    ".bundle",
    ".gradle",
    "out",
    "build",
    "dist",
    "target",
    ".next",
    ".nuxt",
    ".output",
    ".parcel-cache",
    ".turbo",
    ".vite",
    ".nx",
    "coverage",
    ".coverage",
    ".nyc_output",
    ".angular",
    ".svelte-kit",
    ".astro",
    ".docusaurus",
    "DerivedData",
    "Pods",
    ".build",
    "Carthage",
    ".dart_tool",
    ".terraform",
];
