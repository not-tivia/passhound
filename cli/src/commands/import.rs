use anyhow::{Context, Result};
use passhound_core::importer::{
    csv as csv_imp, pipeline, shred,
    Classification, ImportId, Mapping, ParseResult,
};
use passhound_core::Vault;
use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};

#[derive(clap::Args)]
pub struct ImportArgs {
    #[command(subcommand)]
    pub command: ImportCommand,
}

#[derive(clap::Subcommand)]
pub enum ImportCommand {
    /// Import from a CSV file.
    Csv(CsvArgs),
    /// Import from pasted text (file, stdin, or $EDITOR).
    Paste(PasteArgs),
    /// Reverse a previous import by id.
    Undo(UndoArgs),
}

#[derive(clap::Args)]
pub struct UndoArgs {
    /// The import id to reverse (find via `import list` or the message printed after a commit).
    pub id: i64,
}

#[derive(clap::Args)]
pub struct PasteArgs {
    /// Read paste content from this file. If absent, read stdin (if piped) or $EDITOR (if a TTY).
    #[arg(long)]
    pub file: Option<PathBuf>,
    /// Print every classified entry.
    #[arg(long)]
    pub show_conflicts: bool,
    /// Apply the import.
    #[arg(long)]
    pub commit: bool,
    /// Skip the post-commit shred prompt (only relevant when --file was passed).
    #[arg(long)]
    pub no_shred: bool,
}

#[derive(clap::Args)]
pub struct CsvArgs {
    /// Path to the CSV file.
    pub path: PathBuf,
    /// Override auto-detect with explicit `field=ColumnName` pairs (comma-separated).
    /// Example: `--mapping site=Name,password=Pass`
    #[arg(long)]
    pub mapping: Option<String>,
    /// Apply this site name to every row, ignoring any site column. Use for
    /// per-site CSVs (e.g. one file per game/service) where the rows are
    /// just login/password/notes without a site column.
    #[arg(long)]
    pub site: Option<String>,
    /// Print every classified entry (passwords are always redacted).
    #[arg(long)]
    pub show_conflicts: bool,
    /// Apply the import. Without this flag, only the dry-run summary is printed.
    #[arg(long)]
    pub commit: bool,
    /// Skip the post-commit shred prompt.
    #[arg(long)]
    pub no_shred: bool,
}

pub fn run(vault_path: &Path, args: ImportArgs) -> Result<()> {
    match args.command {
        ImportCommand::Csv(a) => run_csv(vault_path, a),
        ImportCommand::Paste(a) => run_paste(vault_path, a),
        ImportCommand::Undo(a) => run_undo(vault_path, a),
    }
}

fn run_csv(vault_path: &Path, args: CsvArgs) -> Result<()> {
    if !vault_path.exists() {
        anyhow::bail!("vault not found at {}", vault_path.display());
    }
    if !args.path.exists() {
        anyhow::bail!("csv not found at {}", args.path.display());
    }

    let pw = rpassword::prompt_password("Master password: ")?;
    let mut vault = Vault::open(vault_path)?;
    vault.unlock(pw.as_bytes()).context("unlock failed")?;

    let explicit_mapping = match args.mapping.as_deref() {
        Some(s) => Some(parse_mapping_arg(s, &args.path)?),
        None => None,
    };

    let site_override = args.site.clone();
    let parse_result = match csv_imp::parse_file(&vault, &args.path, explicit_mapping, site_override.clone()) {
        Ok(r) => r,
        Err(passhound_core::Error::NeedsColumnMapping { headers }) => {
            let mapping = interactive_mapping(&headers)?;
            csv_imp::save_mapping(&vault, &headers, &mapping)?;
            csv_imp::parse_file(&vault, &args.path, Some(mapping), site_override)?
        }
        Err(e) => return Err(e.into()),
    };

    print_diagnostics(&parse_result);
    let preview = pipeline::preview(&vault, parse_result.entries)?;
    print_summary(&preview, parse_result.diagnostics.len());

    if args.show_conflicts {
        print_conflicts(&preview);
    }

    if !args.commit {
        println!("(dry run; rerun with --commit to apply)");
        return Ok(());
    }

    let import_id = pipeline::commit(&vault, preview, "csv", Some(&args.path))?;
    println!("Imported (id={}).", import_id.0);

    if !args.no_shred {
        prompt_and_shred(&args.path)?;
    }
    Ok(())
}

fn run_paste(vault_path: &Path, args: PasteArgs) -> Result<()> {
    if !vault_path.exists() {
        anyhow::bail!("vault not found at {}", vault_path.display());
    }

    let pw = rpassword::prompt_password("Master password: ")?;
    let mut vault = Vault::open(vault_path)?;
    vault.unlock(pw.as_bytes()).context("unlock failed")?;

    let (input_text, source_path): (String, Option<PathBuf>) = match args.file.as_deref() {
        Some(p) => (
            std::fs::read_to_string(p).with_context(|| format!("read {}", p.display()))?,
            Some(p.to_path_buf()),
        ),
        None => {
            use std::io::IsTerminal;
            if std::io::stdin().is_terminal() {
                (read_from_editor()?, None)
            } else {
                let mut buf = String::new();
                std::io::stdin().lock().read_to_string(&mut buf)?;
                (buf, None)
            }
        }
    };

    let parse_result = passhound_core::importer::parse_paste(&input_text);
    print_diagnostics(&parse_result);

    let preview = pipeline::preview(&vault, parse_result.entries)?;
    print_summary(&preview, parse_result.diagnostics.len());

    if args.show_conflicts {
        print_conflicts(&preview);
    }

    if !args.commit {
        println!("(dry run; rerun with --commit to apply)");
        return Ok(());
    }

    let import_id = pipeline::commit(&vault, preview, "paste", source_path.as_deref())?;
    println!("Imported (id={}).", import_id.0);

    if !args.no_shred {
        if let Some(p) = source_path.as_deref() {
            prompt_and_shred(p)?;
        }
    }
    Ok(())
}

fn read_from_editor() -> Result<String> {
    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "nano".to_string());
    let tmp = tempfile::NamedTempFile::new()?;
    let path = tmp.path().to_path_buf();
    let status = std::process::Command::new(&editor)
        .arg(&path)
        .status()
        .with_context(|| format!("launch editor {editor}"))?;
    if !status.success() {
        anyhow::bail!("editor exited with non-zero status");
    }
    let mut content = String::new();
    std::fs::File::open(&path)?.read_to_string(&mut content)?;
    Ok(content)
}

fn parse_mapping_arg(s: &str, path: &Path) -> Result<Mapping> {
    // Read just the header row to translate column-name args to indices.
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_path(path)?;
    let headers: Vec<String> = rdr.headers()?.iter().map(|s| s.to_string()).collect();
    let mut site: Option<usize> = None;
    let mut url: Option<usize> = None;
    let mut username: Option<usize> = None;
    let mut password: Option<usize> = None;
    let mut notes: Option<usize> = None;
    let mut created_at: Option<usize> = None;

    for pair in s.split(',') {
        let mut parts = pair.splitn(2, '=');
        let field = parts.next().unwrap_or("").trim();
        let col = parts
            .next()
            .ok_or_else(|| anyhow::anyhow!("--mapping pair missing '=': {pair}"))?
            .trim();
        let idx = headers
            .iter()
            .position(|h| h.eq_ignore_ascii_case(col))
            .ok_or_else(|| anyhow::anyhow!("column '{col}' not in headers: {headers:?}"))?;
        match field {
            "site" => site = Some(idx),
            "url" => url = Some(idx),
            "username" => username = Some(idx),
            "password" => password = Some(idx),
            "notes" => notes = Some(idx),
            "created_at" => created_at = Some(idx),
            other => anyhow::bail!("unknown mapping field '{other}'"),
        }
    }

    // `site` is optional now — caller may supply `--site NAME` to override.
    Ok(Mapping {
        site,
        url,
        username,
        password: password.ok_or_else(|| anyhow::anyhow!("--mapping must include password=..."))?,
        notes,
        created_at,
    })
}

fn interactive_mapping(headers: &[String]) -> Result<Mapping> {
    println!("\nHeader columns:");
    for (i, h) in headers.iter().enumerate() {
        println!("  [{i}] {h}");
    }
    println!();

    let stdin = std::io::stdin();
    let mut buf = String::new();

    let prompt_idx = |label: &str, required: bool, buf: &mut String| -> Result<Option<usize>> {
        loop {
            print!("Column index for {label}{}: ", if required { " (required)" } else { " (blank to skip)" });
            std::io::stdout().flush()?;
            buf.clear();
            stdin.lock().read_line(buf)?;
            let s = buf.trim();
            if s.is_empty() {
                if required {
                    println!("required.");
                    continue;
                }
                return Ok(None);
            }
            match s.parse::<usize>() {
                Ok(n) if n < headers.len() => return Ok(Some(n)),
                _ => {
                    println!("invalid index.");
                    continue;
                }
            }
        }
    };

    // site is optional — user can leave blank if they plan to pass --site NAME.
    let site = prompt_idx("site", false, &mut buf)?;
    let url = prompt_idx("url", false, &mut buf)?;
    let username = prompt_idx("username", false, &mut buf)?;
    let password = prompt_idx("password", true, &mut buf)?.unwrap();
    let notes = prompt_idx("notes", false, &mut buf)?;
    let created_at = prompt_idx("created_at", false, &mut buf)?;

    Ok(Mapping {
        site,
        url,
        username,
        password,
        notes,
        created_at,
    })
}

fn print_diagnostics(parse: &ParseResult) {
    if !parse.diagnostics.is_empty() {
        println!("Skipped {} row(s) due to parse errors:", parse.diagnostics.len());
        for d in &parse.diagnostics {
            println!("  row {}: {} ({})", d.row, d.reason, truncate(&d.raw, 60));
        }
    }
}

fn print_summary(preview: &pipeline::Preview, skipped: usize) {
    println!(
        "Preview: {} new, {} duplicate(s), {} merge(s), {} skipped.",
        preview.new, preview.duplicates, preview.merges, skipped
    );
}

fn print_conflicts(preview: &pipeline::Preview) {
    println!();
    for item in &preview.items {
        let kind = match item.classification {
            Classification::New => "NEW",
            Classification::DuplicateOfTriple => "DUP",
            Classification::MergeWithNewPassword => "MERGE",
        };
        println!(
            "  [{kind}] site={} user={} password=<redacted>",
            item.entry.site,
            item.entry.username.as_deref().unwrap_or("-"),
        );
    }
}

fn prompt_and_shred(path: &Path) -> Result<()> {
    print!("Shred {}? [Y/n] ", path.display());
    std::io::stdout().flush()?;
    let mut buf = String::new();
    std::io::stdin().lock().read_line(&mut buf)?;
    let answer = buf.trim().to_ascii_lowercase();
    if answer.is_empty() || answer == "y" || answer == "yes" {
        match shred::shred_file(path) {
            Ok(()) => println!("Shredded."),
            Err(e) => println!("Warning: shred failed: {e}"),
        }
    } else {
        println!("(source left in place)");
    }
    Ok(())
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let cut: String = s.chars().take(n).collect();
        format!("{cut}…")
    }
}

fn run_undo(vault_path: &Path, args: UndoArgs) -> Result<()> {
    if !vault_path.exists() {
        anyhow::bail!("vault not found at {}", vault_path.display());
    }
    let pw = rpassword::prompt_password("Master password: ")?;
    let mut vault = Vault::open(vault_path)?;
    vault.unlock(pw.as_bytes()).context("unlock failed")?;

    let counts = match pipeline::undo(&vault, ImportId(args.id)) {
        Ok(c) => c,
        Err(passhound_core::Error::NotFound) => {
            anyhow::bail!("unknown import id {}", args.id);
        }
        Err(e) => return Err(e.into()),
    };
    println!(
        "Undid import {}: deleted {} password(s), {} account(s), {} site(s).",
        args.id, counts.passwords, counts.accounts, counts.sites
    );
    Ok(())
}
