macro_rules! type_default {
    (Vec<$type:ty>) => {
        Vec<$type>
    };
    (Option<$type:ty>) => {
        Option<$type>
    };
    ($type:ty) => {
        Option<$type>
    };
}

macro_rules! type_default_argument {
    (Vec<$type:ty>) => {
        Vec<$type>
    };
    (Option<$type:ty>) => {
        Option<$type>
    };
    ($type:ty) => {
        $type
    };
}

macro_rules! type_default_constructor {
    (Vec<$type:ty>, $val:expr) => {
        $val
    };
    (Option<$type:ty>, $val:expr) => {
        $val
    };
    ($type:ty, $val:expr) => {
        Some($val)
    };
}

macro_rules! builder_importance {
    ($value:ident, Vec<$type:ty>, $key:ident, required($msg:literal)) => {
        let $key = $value.$key;
        if $key.is_empty() {
            bail!("Empty list {} specified. {}", $key, $msg);
        };
    };
    ($value:ident, Vec<$type:ty>, $key:ident, warn($msg:literal)) => {
        if $key.is_empty() {
            log::warn!(target: "surfer-integration", "Empty list {:?} specified. {}", $key, $msg)
        };
    };
    ($value:ident, Option<$type:ty>, $key:ident, required($msg:literal)) => {
        compile_error!("Cannot force option on optional values, remove the Option instead");
    };
    ($value:ident, Option<$type:ty>, $key:ident, warn($msg:literal)) => {
        if $key.is_none() else {
            log::warn!("No {:?} specified. {}", $key, $msg);
        };
    };
    ($value:ident, $type:ty, $key:ident, warn($msg:literal)) => {
        compile_error!("Cannot have warnings on non-optional values, use a type with Option instead");
    };
    ($value:ident, $type:ty, $key:ident, required($msg:literal)) => {
        let Some($key) = $key.take() else {
            bail!("No {:?} specified. {}", $key, $msg);
        };
    };
    ($value:ident, $type:ty, $key:ident, ) => {};
}

macro_rules! auto_builder {
    ($name:ident <- $builder:ident ($propenum:ident) = $($var:ident ($($type:tt)+) $($importance:ident($msg:literal))? ),*) => {
        #[derive(Clone, Debug, Default, Serialize)]
        pub(crate) struct $name {
            $(
                pub $var: type_default_argument!($($type)+),
            )*
        }

        #[derive(Clone, Debug, Default, Serialize)]
        pub(crate) struct $builder {
            $(
                pub $var: type_default!($($type)+),
            )*
        }

        paste! {
            pub(crate) enum $propenum {
                $(
                    [<$var:camel>],
                )*
            }
        }

        paste! {
            impl TryFrom<&str> for $propenum {
                type Error = anyhow::Error;

                fn try_from(value: &str) -> Result<Self, Self::Error> {
                    match value {
                        $(
                            stringify!($var) => Ok(Self::[<$var:camel>]),
                        )*
                        _ => bail!(stringify!("String does not match any of the following: ", $($var, )*))
                    }
                }
            }
        }

        paste! {
            impl $builder {
                $(
                    pub fn [<with_ $var>](mut self, value: type_default_argument!($($type)+)) -> Self {
                        self.$var = type_default_constructor!($($type)+, value);
                        self
                    }
                )*
            }
        }

        impl TryFrom<$builder> for $name {
            type Error = anyhow::Error;

            fn try_from(value: $builder) -> Result<Self, Self::Error> {
                $(
                    #[allow(unused_mut)]
                    let mut $var = value.$var;
                )*

                $(
                    // $(
                        builder_importance!(value, $($type)+, $var, $( $importance($msg) )?);
                    // )?
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
