use std::path::Path;
use std::process::Command;

pub trait CommandHomeExt {
    fn isolated_home(&mut self, home: &Path) -> &mut Self;
}

impl CommandHomeExt for Command {
    fn isolated_home(&mut self, home: &Path) -> &mut Self {
        let config = home.join(".test-xdg/config");
        let data = home.join(".test-xdg/data");
        let state = home.join(".test-xdg/state");
        let cache = home.join(".test-xdg/cache");
        for path in [&config, &data, &state, &cache] {
            std::fs::create_dir_all(path).expect("create temporary XDG directory");
        }
        self.env("HOME", home)
            .env("XDG_CONFIG_HOME", config)
            .env("XDG_DATA_HOME", data)
            .env("XDG_STATE_HOME", state)
            .env("XDG_CACHE_HOME", cache)
    }
}

#[allow(dead_code)]
pub fn output(args: &[&str]) -> std::process::Output {
    let home = tempfile::tempdir().expect("temporary HOME");
    let mut command = Command::new(env!("CARGO_BIN_EXE_mole-linux"));
    command.args(args);
    command.isolated_home(home.path());
    command.output().expect("run isolated mole-linux")
}
