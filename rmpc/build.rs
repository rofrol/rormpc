include!("src/config/cli.rs");

use std::{error::Error, fs};

use clap::{Command as ClapCommand, CommandFactory};
use clap_complete::{
    Shell::{Bash, Fish, Zsh},
    generate_to,
};
use clap_mangen::Man;
use vergen_gitcl::{Emitter, GitclBuilder};

static NAME: &str = "rormpc"; // rormpc: completions and man page for the renamed binary

fn generate_man_pages(cmd: ClapCommand) -> Result<(), Box<dyn Error>> {
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("Parent path for CARGO_MANIFEST_DIR to be exist")
        .join("target")
        .join("man");
    let mut buffer = Vec::default();

    Man::new(cmd).render(&mut buffer)?;
    fs::create_dir_all(&out)?;
    fs::write(out.join(NAME.to_owned() + ".1"), buffer)?;
    Ok(())
}

fn generate_shell_completions(mut cmd: ClapCommand) -> Result<(), Box<dyn Error>> {
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("Parent path for CARGO_MANIFEST_DIR to be exist")
        .join("target")
        .join("completions");

    std::fs::create_dir_all(&out)?;
    for shell in [Bash, Fish, Zsh] {
        generate_to(shell, &mut cmd, NAME, &out)?;
    }
    Ok(())
}

fn emit_git_info() -> Result<(), Box<dyn Error>> {
    Emitter::default()
        // rormpc: sha + dirty + subject feed the Status(BuildRevision) theme property
        .add_instructions(&GitclBuilder::default().describe(false, false, None).sha(true).dirty(false).commit_message(true).build()?)?
        .emit()?;

    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut cmd = Args::command();
    cmd.set_bin_name(NAME);

    generate_man_pages(cmd.clone())?;
    generate_shell_completions(cmd)?;

    emit_git_info()?;

    Ok(())
}
