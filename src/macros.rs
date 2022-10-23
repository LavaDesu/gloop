#[macro_export]
macro_rules! cmdmatch {
    ($ctx:ident, $int:ident, [$($cmd:tt$([$cname:literal])?),+ $(,)?]) => {
        match $int.data.name.as_str() {
            $( cmdmatch!{@inner $cmd$(, $cname)?} => $cmd::run(&$ctx, &$int).await, )*
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
    ($builder:ident, [$($cmd:tt),+ $(,)?]) => {
        $builder$( .create_application_command(|cmd| $cmd::register(cmd)) )*
    };
}

#[macro_export]
macro_rules! data_scope {
    ($ctx:expr, $($name:ident = $type:ident),+, {$($token:tt)*}) => {{
        let data = $ctx.data.read().await;
        $( let $name = data.get::<$type>().unwrap(); )*
        $( $token )*
    }};
}

#[macro_export]
macro_rules! get_data {
    ($data:expr, $type:ident) => {
        $data.get::<$type>().unwrap()
    };
    ($data:expr, $($type:ident),+ $(,)?) => {
        ($( $data.get::<$type>().unwrap(), )*)
    };
}

#[macro_export]
macro_rules! intr_msg {
    ($int:ident, $http:ident, $cnt:expr) => {
        intr_data!($int, $http, |d| d.content($cnt));
    };
}

#[macro_export]
macro_rules! intr_emsg {
    ($int:ident, $http:ident, $cnt:expr) => {
        intr_data!($int, $http, |d| d.content($cnt).ephemeral(true))
    };
}

#[macro_export]
macro_rules! intr_data {
    ($int:ident, $http:ident, $($data:tt)*) => {
        $int.create_interaction_response(&$http, |resp| {
            resp.kind(InteractionResponseType::ChannelMessageWithSource)
                .interaction_response_data($($data)*)
        })
    };
}
