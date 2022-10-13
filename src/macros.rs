/*macro_rules! pgeneral {
    ($color:expr, $fmt:expr$(, $($arg:tt)*)?) => {{
        println!(concat!("\x1b[{}m[{}]\x1b[0m ", $fmt), $color, chrono::prelude::Local::now().time().format("%H:%M:%S"), $($($arg)*)?);
    }};
}

#[macro_export]
macro_rules! pinfo {
    ($($arg:tt)+) => {
        pgeneral!(34, $($arg)*)
    };
}*/

#[macro_export]
macro_rules! cmdmatch {
    ($ctx:ident, $int:ident, [$($cmd:ident$(($cname:literal))?),+ $(,)?]) => {
        match $int.data.name.as_str() {
            $( cmdmatch!{@inner $cmd$(, $cname)?} => commands::$cmd::run(&$ctx, &$int).await, )*
            n => Err(anyhow::anyhow!(format!("command '{}' not found", n)).into())
        }
    };
    (@inner $one:ident) => {
        stringify!($one)
    };
    (@inner $one:ident, $two:literal) => {
        $two
    };
}

#[macro_export]
macro_rules! cmdcreate {
    ($builder:ident, [$($cname:ident),+ $(,)?]) => {
        $builder$( .create_application_command(|cmd| commands::$cname::register(cmd)) )*
    };
}
