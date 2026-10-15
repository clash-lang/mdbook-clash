macro_rules! type_default {
    (v<$type:ty>) => {
        Vec<$type>
    };
    (o<$type:ty>) => {
        Option<$type>
    };
    (p<$type:ty>) => {
        Option<$type>
    };
}

macro_rules! type_default_argument {
    (v<$type:ty>) => {
        Vec<$type>
    };
    (o<$type:ty>) => {
        Option<$type>
    };
    (p<$type:ty>) => {
        $type
    };
}

macro_rules! type_default_constructor {
    (v<$type:ty>, $val:expr) => {
        $val
    };
    (o<$type:ty>, $val:expr) => {
        $val
    };
    (p<$type:ty>, $val:expr) => {
        Some($val)
    };
}

macro_rules! builder_importance {
    ($value:ident, p, $key:ident, required($msg:literal)) => {
        let Some($key) = $key.take() else {
            bail!("No {:?} specified. {}", $key, $msg);
        };
    };
    ($value:ident, v, $key:ident, required($msg:literal)) => {
        let $key = $value.$key;
        if $key.is_empty() {
            bail!("Empty list {} specified. {}", $key, $msg);
        };
    };
    ($value:ident, p, $key:ident, warn($msg:literal)) => {
        compile_error!("Cannot have warnings on p values, prefix with o instead");
    };
    ($value:ident, v, $key:ident, warn($msg:literal)) => {
        if $key.is_empty() {
            log::warn!(target: "surfer-integration", "Empty list {:?} specified. {}", $key, $msg)
        };
    };
    ($value:ident, o, $key:ident, required($msg:literal)) => {
        compile_error!("Cannot force option on o values, prefix with p instead");
    };
    ($value:ident, o, $key:ident, warn($msg:literal)) => {
        if $key.is_none() else {
            log::warn!("No {:?} specified. {}", $key, $msg);
        };
    };
}

macro_rules! auto_builder {
    ($name:ident <- $builder:ident = $($var:ident: $prefix:ident<$type:ty> $($importance:ident($msg:literal))? ),*) => {
        #[derive(Clone, Debug, Default, Serialize)]
        pub(crate) struct $name {
            $(
                pub $var: type_default_argument!($prefix<$type>),
            )*
        }

        #[derive(Clone, Debug, Default, Serialize)]
        pub(crate) struct $builder {
            $(
                pub $var: type_default!($prefix<$type>),
            )*
        }

        impl $builder {
            $(
                concat_idents!(fn_name = with_, $var {
                    pub fn fn_name(mut self, value: type_default_argument!($prefix<$type>)) -> Self {
                        self.$var = type_default_constructor!($prefix<$type>, value);
                        self
                    }
                });
            )*
        }

        impl TryFrom<$builder> for $name {
            type Error = anyhow::Error;

            fn try_from(value: $builder) -> Result<Self, Self::Error> {
                $(
                    #[allow(unused_mut)]
                    let mut $var = value.$var;
                )*

                $(
                    $(
                        builder_importance!(value, $prefix, $var, $importance($msg));
                    )?
                )*

                Ok(Self {
                    $(
                        $var,
                    )*
                })
            }
        }
    };
}

pub(crate) use builder_importance;
pub(crate) use type_default;
pub(crate) use type_default_argument;
pub(crate) use type_default_constructor;
pub(crate) use auto_builder;
