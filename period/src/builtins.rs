use std::cell::RefCell;
use std::io::{self, Write};
use std::rc::Rc;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use num_bigint::BigInt;
use num_traits::cast::{FromPrimitive, ToPrimitive};

use crate::environment::Environment;
use crate::value::{range_len, BuiltInValue, Integer, Value};

pub fn install_builtins(env: &Environment) {
    env.define_untyped("length", Value::BuiltIn(Box::new(BuiltInValue { name: "length".to_string(), min_arity: 1, max_arity: 1, func: builtin_length })));
    env.define_untyped("string", Value::BuiltIn(Box::new(BuiltInValue { name: "string".to_string(), min_arity: 1, max_arity: 1, func: builtin_string })));
    env.define_untyped("number", Value::BuiltIn(Box::new(BuiltInValue { name: "number".to_string(), min_arity: 1, max_arity: 1, func: builtin_number })));
    env.define_untyped("integer", Value::BuiltIn(Box::new(BuiltInValue { name: "integer".to_string(), min_arity: 1, max_arity: 1, func: builtin_integer })));
    env.define_untyped("boolean", Value::BuiltIn(Box::new(BuiltInValue { name: "boolean".to_string(), min_arity: 1, max_arity: 1, func: builtin_boolean })));
    env.define_untyped("type", Value::BuiltIn(Box::new(BuiltInValue { name: "type".to_string(), min_arity: 1, max_arity: 1, func: builtin_type })));
    env.define_untyped("input", Value::BuiltIn(Box::new(BuiltInValue { name: "input".to_string(), min_arity: 0, max_arity: 0, func: builtin_input })));
    env.define_untyped("range", Value::BuiltIn(Box::new(BuiltInValue { name: "range".to_string(), min_arity: 1, max_arity: 3, func: builtin_range })));
    env.define_untyped("error", Value::BuiltIn(Box::new(BuiltInValue { name: "error".to_string(), min_arity: 1, max_arity: 1, func: builtin_error })));
    env.define_untyped("append", Value::BuiltIn(Box::new(BuiltInValue { name: "append".to_string(), min_arity: 2, max_arity: 2, func: builtin_append })));
}

fn builtin_append(args: &[Value]) -> Result<Value, String> {
    match &args[0] {
        Value::List(l) => {
            l.borrow_mut().push(args[1].clone());
            Ok(Value::Nothing)
        }
        _ => Err("append requires a list".to_string()),
    }
}

fn builtin_length(args: &[Value]) -> Result<Value, String> {
    match &args[0] {
        Value::String(s) => Ok(Value::integer(s.chars().count() as i64)),
        Value::List(l) => Ok(Value::integer(l.borrow().len() as i64)),
        Value::Dict(d) => Ok(Value::integer(d.borrow().len() as i64)),
        Value::Range { start, stop, step } => Ok(Value::big_integer(range_len(start, stop, step))),
        _ => Err("Cannot get length".to_string()),
    }
}

fn builtin_string(args: &[Value]) -> Result<Value, String> {
    Ok(Value::String(args[0].to_string()))
}

fn builtin_number(args: &[Value]) -> Result<Value, String> {
    match &args[0] {
        Value::Integer(n) => Ok(Value::Number(n.to_f64())),
        Value::Number(n) => Ok(Value::Number(*n)),
        Value::String(s) => s.parse::<f64>().map(Value::Number).or_else(|_| s.parse::<BigInt>().map(Value::big_integer)).map_err(|_| "Cannot convert to number".to_string()),
        Value::Bool(true) => Ok(Value::Number(1.0)),
        Value::Bool(false) => Ok(Value::Number(0.0)),
        _ => Err("Cannot convert to number".to_string()),
    }
}

fn builtin_integer(args: &[Value]) -> Result<Value, String> {
    match &args[0] {
        Value::Integer(n) => Ok(Value::Integer(n.clone())),
        Value::Number(n) => Ok(Value::integer(*n as i64)),
        Value::String(s) => s.parse::<BigInt>().map(Value::big_integer).map_err(|_| "Cannot convert to integer".to_string()),
        Value::Bool(true) => Ok(Value::integer(1)),
        Value::Bool(false) => Ok(Value::integer(0)),
        _ => Err("Cannot convert to integer".to_string()),
    }
}

fn builtin_boolean(args: &[Value]) -> Result<Value, String> {
    let b = match &args[0] {
        Value::Bool(b) => *b,
        Value::Integer(n) => !n.is_zero(),
        Value::Number(n) => *n != 0.0,
        Value::String(s) => !s.is_empty(),
        Value::Nothing => false,
        Value::List(l) => !l.borrow().is_empty(),
        Value::Dict(d) => !d.borrow().is_empty(),
        Value::Range { start, stop, step } => {
            let zero = Integer::Small(0);
            (step > &zero && start < stop) || (step < &zero && start > stop)
        }
        _ => true,
    };
    Ok(Value::Bool(b))
}

fn builtin_type(args: &[Value]) -> Result<Value, String> {
    Ok(Value::String(args[0].type_name().to_string()))
}

fn builtin_input(_: &[Value]) -> Result<Value, String> {
    let mut s = String::new();
    io::stdout().flush().map_err(|e| format!("cannot flush stdout: {}", e))?;
    io::stdin().read_line(&mut s).map_err(|e| e.to_string())?;
    Ok(Value::String(s.trim_end().to_string()))
}

fn builtin_error(args: &[Value]) -> Result<Value, String> {
    Err(match &args[0] { Value::String(s) => s.clone(), v => v.to_string() })
}

fn builtin_range(args: &[Value]) -> Result<Value, String> {
    let to_i = |v: &Value| match v {
        Value::Integer(n) => Ok(n.clone()),
        Value::Number(n) => {
            if n.fract() != 0.0 {
                return Err("range arguments must be whole numbers".to_string());
            }
            BigInt::from_f64(*n)
                .map(Integer::from_bigint)
                .ok_or_else(|| "range arguments must be finite".to_string())
        }
        _ => Err("range args must be integers".to_string()),
    };
    let (start, stop, step) = match args.len() {
        1 => (Integer::Small(0), to_i(&args[0])?, Integer::Small(1)),
        2 => (to_i(&args[0])?, to_i(&args[1])?, Integer::Small(1)),
        3 => (to_i(&args[0])?, to_i(&args[1])?, to_i(&args[2])?),
        _ => unreachable!(),
    };
    if step.is_zero() { return Err("range step cannot be zero".to_string()); }
    Ok(Value::Range { start, stop, step })
}

fn make_module(values: Vec<(&str, Value)>) -> Rc<RefCell<Environment>> {
    let env = Environment::new();
    for (name, value) in values { env.borrow().define_untyped(name, value); }
    env
}

pub fn make_math_module() -> Rc<RefCell<Environment>> {
    macro_rules! unary_float {
        ($name:ident, $f:path) => {
            fn $name(args: &[Value]) -> Result<Value, String> {
                let n = match &args[0] { Value::Integer(i) => i.to_f64(), Value::Number(n) => *n, _ => return Err("expected number".to_string()) };
                Ok(Value::Number($f(n)))
            }
        };
    }
    unary_float!(sin_fn, f64::sin);
    unary_float!(cos_fn, f64::cos);
    unary_float!(tan_fn, f64::tan);
    unary_float!(sqrt_fn, f64::sqrt);
    unary_float!(abs_fn, f64::abs);
    unary_float!(floor_fn, f64::floor);
    unary_float!(ceil_fn, f64::ceil);
    make_module(vec![
        ("sin", Value::BuiltIn(Box::new(BuiltInValue { name: "sin".to_string(), min_arity: 1, max_arity: 1, func: sin_fn }))),
        ("cos", Value::BuiltIn(Box::new(BuiltInValue { name: "cos".to_string(), min_arity: 1, max_arity: 1, func: cos_fn }))),
        ("tan", Value::BuiltIn(Box::new(BuiltInValue { name: "tan".to_string(), min_arity: 1, max_arity: 1, func: tan_fn }))),
        ("sqrt", Value::BuiltIn(Box::new(BuiltInValue { name: "sqrt".to_string(), min_arity: 1, max_arity: 1, func: sqrt_fn }))),
        ("abs", Value::BuiltIn(Box::new(BuiltInValue { name: "abs".to_string(), min_arity: 1, max_arity: 1, func: abs_fn }))),
        ("floor", Value::BuiltIn(Box::new(BuiltInValue { name: "floor".to_string(), min_arity: 1, max_arity: 1, func: floor_fn }))),
        ("ceil", Value::BuiltIn(Box::new(BuiltInValue { name: "ceil".to_string(), min_arity: 1, max_arity: 1, func: ceil_fn }))),
        ("pi", Value::Number(std::f64::consts::PI)),
    ])
}

pub fn make_random_module() -> Rc<RefCell<Environment>> {
    static STATE: Mutex<(bool, u64)> = Mutex::new((false, 0));
    fn random_fn(_: &[Value]) -> Result<Value, String> {
        let mut guard = STATE.lock().map_err(|e| format!("random state poisoned: {}", e))?;
        let (seeded, seed) = &mut *guard;
        if !*seeded {
            *seed = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .map_err(|e| format!("system clock error: {}", e))?;
            *seeded = true;
        }
        *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let r = ((*seed >> 33) as f64) / ((1u64 << 31) as f64);
        Ok(Value::Number(r))
    }
    fn seed_fn(args: &[Value]) -> Result<Value, String> {
        let n = match &args[0] {
            Value::Integer(i) => i.to_bigint(),
            Value::Number(n) if n.fract() == 0.0 => BigInt::from_f64(*n)
                .ok_or_else(|| "seed must be a finite number".to_string())?,
            _ => return Err("seed must be an integer".to_string()),
        };
        // Reduce to the low 64 bits so any integer, however large, is accepted.
        let seed = (n & BigInt::from(u64::MAX)).to_u64().unwrap_or(0);
        let mut guard = STATE.lock().map_err(|e| format!("random state poisoned: {}", e))?;
        guard.0 = true;
        guard.1 = seed;
        Ok(Value::Nothing)
    }
    make_module(vec![
        ("random", Value::BuiltIn(Box::new(BuiltInValue { name: "random".to_string(), min_arity: 0, max_arity: 0, func: random_fn }))),
        ("seed", Value::BuiltIn(Box::new(BuiltInValue { name: "seed".to_string(), min_arity: 1, max_arity: 1, func: seed_fn }))),
    ])
}

pub fn make_string_module() -> Rc<RefCell<Environment>> {
    make_module(vec![
        ("upper", Value::BuiltIn(Box::new(BuiltInValue { name: "upper".to_string(), min_arity: 1, max_arity: 1, func: |args| {
            match &args[0] { Value::String(s) => Ok(Value::String(s.to_uppercase())), _ => Err("expected string".to_string()) }
        }}))),
        ("lower", Value::BuiltIn(Box::new(BuiltInValue { name: "lower".to_string(), min_arity: 1, max_arity: 1, func: |args| {
            match &args[0] { Value::String(s) => Ok(Value::String(s.to_lowercase())), _ => Err("expected string".to_string()) }
        }}))),
        ("trim", Value::BuiltIn(Box::new(BuiltInValue { name: "trim".to_string(), min_arity: 1, max_arity: 1, func: |args| {
            match &args[0] { Value::String(s) => Ok(Value::String(s.trim().to_string())), _ => Err("expected string".to_string()) }
        }}))),
        ("split", Value::BuiltIn(Box::new(BuiltInValue { name: "split".to_string(), min_arity: 2, max_arity: 2, func: |args| {
            match (&args[0], &args[1]) {
                (Value::String(s), Value::String(delim)) => {
                    let parts: Vec<Value> = s.split(delim).map(|p| Value::String(p.to_string())).collect();
                    Ok(Value::List(Rc::new(RefCell::new(parts))))
                }
                _ => Err("expected string and delimiter".to_string())
            }
        }}))),
        ("contains", Value::BuiltIn(Box::new(BuiltInValue { name: "contains".to_string(), min_arity: 2, max_arity: 2, func: |args| {
            match (&args[0], &args[1]) {
                (Value::String(s), Value::String(sub)) => Ok(Value::Bool(s.contains(sub))),
                _ => Err("expected string and substring".to_string())
            }
        }}))),
        ("starts_with", Value::BuiltIn(Box::new(BuiltInValue { name: "starts_with".to_string(), min_arity: 2, max_arity: 2, func: |args| {
            match (&args[0], &args[1]) {
                (Value::String(s), Value::String(prefix)) => Ok(Value::Bool(s.starts_with(prefix))),
                _ => Err("expected string and prefix".to_string())
            }
        }}))),
        ("ends_with", Value::BuiltIn(Box::new(BuiltInValue { name: "ends_with".to_string(), min_arity: 2, max_arity: 2, func: |args| {
            match (&args[0], &args[1]) {
                (Value::String(s), Value::String(suffix)) => Ok(Value::Bool(s.ends_with(suffix))),
                _ => Err("expected string and suffix".to_string())
            }
        }}))),
        ("replace", Value::BuiltIn(Box::new(BuiltInValue { name: "replace".to_string(), min_arity: 3, max_arity: 3, func: |args| {
            match (&args[0], &args[1], &args[2]) {
                (Value::String(s), Value::String(from), Value::String(to)) => Ok(Value::String(s.replace(from, to))),
                _ => Err("expected string, from, and to".to_string())
            }
        }}))),
        ("slice", Value::BuiltIn(Box::new(BuiltInValue { name: "slice".to_string(), min_arity: 2, max_arity: 2, func: |args| {
            match (&args[0], &args[1]) {
                (Value::String(s), Value::Integer(start)) => {
                    let chars: Vec<char> = s.chars().collect();
                    let len = chars.len();
                    let start = start.to_i64().ok_or("slice start too large")?;
                    let start = if start < 0 { len.saturating_sub((-start) as usize) } else { start as usize };
                    let start = start.min(len);
                    Ok(Value::String(chars[start..].iter().collect()))
                }
                _ => Err("expected string and integer start".to_string())
            }
        }}))),
        ("substring", Value::BuiltIn(Box::new(BuiltInValue { name: "substring".to_string(), min_arity: 3, max_arity: 3, func: |args| {
            match (&args[0], &args[1], &args[2]) {
                (Value::String(s), Value::Integer(start), Value::Integer(end)) => {
                    let chars: Vec<char> = s.chars().collect();
                    let len = chars.len();
                    let start = start.to_i64().ok_or("substring start too large")?;
                    let start = if start < 0 { len.saturating_sub((-start) as usize) } else { start as usize };
                    let end = end.to_i64().ok_or("substring end too large")?;
                    let end = if end < 0 { len.saturating_sub((-end) as usize) } else { end as usize };
                    let end = end.min(len);
                    let start = start.min(end);
                    Ok(Value::String(chars[start..end].iter().collect()))
                }
                _ => Err("expected string, integer start, and integer end".to_string())
            }
        }}))),
    ])
}

pub fn make_time_module() -> Rc<RefCell<Environment>> {
    make_module(vec![
        ("now", Value::BuiltIn(Box::new(BuiltInValue { name: "now".to_string(), min_arity: 0, max_arity: 0, func: |_| {
            let secs = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .map_err(|e| format!("system clock error: {}", e))?;
            Ok(Value::Number(secs))
        }}))),
    ])
}

fn expect_string_arg(arg: &Value, what: &str) -> Result<String, String> {
    match arg {
        Value::String(s) => Ok(s.clone()),
        _ => Err(format!("expected string for {}", what)),
    }
}

/// Show a modal message box. Returns the raw button code on Windows
/// (IDOK=1, IDYES=6, IDNO=7); on other platforms returns 1 for the
/// affirmative/OK button and 2 for cancel/no, or an error if no dialog
/// tool is available.
#[cfg(target_os = "windows")]
fn platform_message_box(message: &str, yes_no: bool) -> Result<i32, String> {
    use std::ffi::c_void;
    #[link(name = "user32")]
    unsafe extern "system" {
        fn MessageBoxW(hwnd: *mut c_void, text: *const u16, caption: *const u16, utype: u32) -> i32;
    }
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }
    let text = wide(message);
    let caption = wide("Period");
    const MB_OK: u32 = 0;
    const MB_YESNO: u32 = 4;
    Ok(unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            caption.as_ptr(),
            if yes_no { MB_YESNO } else { MB_OK },
        )
    })
}

#[cfg(target_os = "macos")]
fn platform_message_box(message: &str, yes_no: bool) -> Result<i32, String> {
    let escaped = message.replace('\\', "\\\\").replace('"', "\\\"");
    let script = if yes_no {
        format!(
            "display dialog \"{}\" buttons {{\"No\", \"Yes\"}} default button \"Yes\" with title \"Period\"",
            escaped
        )
    } else {
        format!("display dialog \"{}\" buttons {{\"OK\"}} with title \"Period\"", escaped)
    };
    let output = std::process::Command::new("osascript")
        .args(["-e", &script])
        .output()
        .map_err(|e| format!("cannot show dialog: {}", e))?;
    if !output.status.success() {
        // Non-zero exit means the user cancelled.
        return Ok(2);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(if yes_no && !stdout.contains("Yes") { 2 } else { 1 })
}

#[cfg(all(unix, not(target_os = "macos")))]
fn platform_message_box(message: &str, yes_no: bool) -> Result<i32, String> {
    let mut cmd = std::process::Command::new("zenity");
    if yes_no {
        cmd.args(["--question", "--text", message]);
    } else {
        cmd.args(["--info", "--text", message]);
    }
    let status = cmd
        .status()
        .map_err(|e| format!("cannot show dialog (zenity required): {}", e))?;
    Ok(if status.success() { 1 } else { 2 })
}

#[cfg(target_os = "windows")]
fn platform_notify(title: &str, message: &str) -> Result<(), String> {
    // WinRT toast via PowerShell; the notifier reuses Windows PowerShell's
    // AppUserModelID so no app registration is needed.
    let ps_escape = |s: &str| s.replace('\'', "''");
    let script = format!(
        "[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] | Out-Null; \
         $t = [Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent([Windows.UI.Notifications.ToastTemplateType]::ToastText02); \
         $n = $t.GetElementsByTagName('text'); \
         $n.Item(0).AppendChild($t.CreateTextNode('{}')) | Out-Null; \
         $n.Item(1).AppendChild($t.CreateTextNode('{}')) | Out-Null; \
         $toast = [Windows.UI.Notifications.ToastNotification]::new($t); \
         [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('{{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}}\\WindowsPowerShell\\v1.0\\powershell.exe').Show($toast)",
        ps_escape(title),
        ps_escape(message)
    );
    let status = std::process::Command::new("powershell")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &script])
        .status();
    match status {
        Ok(s) if s.success() => Ok(()),
        // Fall back to a message box when WinRT is unavailable.
        _ => platform_message_box(&format!("{}\n\n{}", title, message), false).map(|_| ()),
    }
}

#[cfg(target_os = "macos")]
fn platform_notify(title: &str, message: &str) -> Result<(), String> {
    let script = format!(
        "display notification \"{}\" with title \"{}\"",
        message.replace('\\', "\\\\").replace('"', "\\\""),
        title.replace('\\', "\\\\").replace('"', "\\\"")
    );
    std::process::Command::new("osascript")
        .args(["-e", &script])
        .status()
        .map_err(|e| format!("cannot show notification: {}", e))?;
    Ok(())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn platform_notify(title: &str, message: &str) -> Result<(), String> {
    std::process::Command::new("notify-send")
        .args([title, message])
        .status()
        .map_err(|e| format!("cannot show notification (notify-send required): {}", e))?;
    Ok(())
}

pub fn make_system_module() -> Rc<RefCell<Environment>> {
    fn run_fn(args: &[Value]) -> Result<Value, String> {
        let command = expect_string_arg(&args[0], "command")?;

        // Safety guard for the teaching environment: shell command execution is
        // disabled by default. Users must explicitly opt in by setting
        // PERIOD_ALLOW_SYSTEM_RUN=1 in the environment.
        let allowed = std::env::var("PERIOD_ALLOW_SYSTEM_RUN")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes"))
            .unwrap_or(false);
        if !allowed {
            return Err(
                "system.run is disabled by default for safety. \
                 Set PERIOD_ALLOW_SYSTEM_RUN=1 to enable shell command execution."
                    .to_string(),
            );
        }

        let output = if cfg!(target_os = "windows") {
            std::process::Command::new("cmd").args(["/C", &command]).output()
        } else {
            std::process::Command::new("sh").args(["-c", &command]).output()
        }
        .map_err(|e| format!("cannot run command: {}", e))?;
        let mut out = String::from_utf8_lossy(&output.stdout).into_owned();
        while out.ends_with('\n') || out.ends_with('\r') {
            out.pop();
        }
        Ok(Value::String(out))
    }

    fn open_fn(args: &[Value]) -> Result<Value, String> {
        let target = expect_string_arg(&args[0], "target")?;
        let result = if cfg!(target_os = "windows") {
            std::process::Command::new("cmd").args(["/C", "start", "", &target]).spawn()
        } else if cfg!(target_os = "macos") {
            std::process::Command::new("open").arg(&target).spawn()
        } else {
            std::process::Command::new("xdg-open").arg(&target).spawn()
        };
        result.map_err(|e| format!("cannot open '{}': {}", target, e))?;
        Ok(Value::Nothing)
    }

    fn alert_fn(args: &[Value]) -> Result<Value, String> {
        let message = expect_string_arg(&args[0], "message")?;
        platform_message_box(&message, false)?;
        Ok(Value::Nothing)
    }

    fn confirm_fn(args: &[Value]) -> Result<Value, String> {
        let message = expect_string_arg(&args[0], "message")?;
        let code = platform_message_box(&message, true)?;
        // IDOK=1 / IDYES=6 on Windows; 1 is the affirmative button elsewhere.
        Ok(Value::Bool(code == 1 || code == 6))
    }

    fn notify_fn(args: &[Value]) -> Result<Value, String> {
        let title = expect_string_arg(&args[0], "title")?;
        let message = expect_string_arg(&args[1], "message")?;
        platform_notify(&title, &message)?;
        Ok(Value::Nothing)
    }

    make_module(vec![
        ("run", Value::BuiltIn(Box::new(BuiltInValue { name: "run".to_string(), min_arity: 1, max_arity: 1, func: run_fn }))),
        ("open", Value::BuiltIn(Box::new(BuiltInValue { name: "open".to_string(), min_arity: 1, max_arity: 1, func: open_fn }))),
        ("alert", Value::BuiltIn(Box::new(BuiltInValue { name: "alert".to_string(), min_arity: 1, max_arity: 1, func: alert_fn }))),
        ("confirm", Value::BuiltIn(Box::new(BuiltInValue { name: "confirm".to_string(), min_arity: 1, max_arity: 1, func: confirm_fn }))),
        ("notify", Value::BuiltIn(Box::new(BuiltInValue { name: "notify".to_string(), min_arity: 2, max_arity: 2, func: notify_fn }))),
    ])
}
