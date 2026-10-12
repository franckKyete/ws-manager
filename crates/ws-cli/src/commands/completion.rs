use ws_core::completion::{generate_completion_script, install_completion, query_completions};
use ws_core::output::OutputHandler;

pub fn execute_completion(shell: Option<&str>, mut install: bool) -> Result<(), String> {
    let mut sh = shell;
    if sh == Some("install") {
        install = true;
        sh = None;
    }

    if install {
        let (ok, msg) = install_completion(sh).map_err(|e| e.to_string())?;
        if ok {
            OutputHandler::print_success(&msg);
        } else {
            OutputHandler::print_warning(&msg);
        }
        return Ok(());
    }

    let resolved_sh = sh.unwrap_or("zsh");
    let script = generate_completion_script(resolved_sh)?;
    print!("{}", script);
    Ok(())
}

pub fn execute_internal_complete(query_type: &str, target: Option<&str>) -> Result<(), String> {
    let items = query_completions(query_type, target);
    for item in items {
        println!("{}", item);
    }
    Ok(())
}
