//! Shell completion and man page generation command

use clap::CommandFactory;
use clap_complete::{Shell, generate};
use std::path::Path;

pub fn run(shell: Shell) {
    let mut cmd = crate::Cli::command();
    let name = cmd.get_name().to_string();
    generate(shell, &mut cmd, name, &mut std::io::stdout());
}

/// Generate man pages for the CLI
pub fn generate_man_pages(output_dir: &Path) -> std::io::Result<()> {
    use clap_mangen::Man;
    use std::fs;

    fs::create_dir_all(output_dir)?;

    let cmd = crate::Cli::command();
    let man = Man::new(cmd.clone());
    let mut buffer = Vec::new();
    man.render(&mut buffer)?;

    let man_path = output_dir.join("gestura.1");
    fs::write(&man_path, buffer)?;
    println!("Generated: {}", man_path.display());

    // Generate man pages for subcommands
    for subcommand in cmd.get_subcommands() {
        if subcommand.get_name() == "help" {
            continue;
        }
        let subman = Man::new(subcommand.clone());
        let mut buffer = Vec::new();
        subman.render(&mut buffer)?;

        let subman_path = output_dir.join(format!("gestura-{}.1", subcommand.get_name()));
        fs::write(&subman_path, buffer)?;
        println!("Generated: {}", subman_path.display());
    }

    Ok(())
}
