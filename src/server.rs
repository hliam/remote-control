//! The server, responses, and requests.

use std::borrow::Cow;
use std::convert::{TryFrom, TryInto};
use std::error::Error;
use std::fmt;
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, Shutdown, SocketAddrV4, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha512};

// TODO: add more granular logging

trait DurationExt {
    /// The amount of time elapsed since the unix epoch.
    fn since_unix_epoch() -> Self;
}
impl DurationExt for Duration {
    #[must_use]
    fn since_unix_epoch() -> Self {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
    }
}

/// An extension trait for `Response` and logging features on `Result`.
pub trait ResultExt<T, E> {
    /// Creates a `Response` of just a status code from this result.
    ///
    /// The status code is set to 200 if `Ok`, otherwise the status code is `err_status_code`.
    #[must_use]
    fn to_status_response(&self, err_status_code: u16) -> Response;
    /// Logs that the connection was refused to `logger` if `Err`. Does nothing if `Ok`.
    ///
    /// The contained error is used as the log message.
    fn log_connection_refused(self, logger: &impl Logger) -> Self;
}

impl<T, E: fmt::Display> ResultExt<T, E> for Result<T, E> {
    fn to_status_response(&self, err_status_code: u16) -> Response {
        Response::from_status(self.as_ref().map_or(err_status_code, |_| 200))
    }

    fn log_connection_refused(self, logger: &impl Logger) -> Self {
        if let Err(e) = &self {
            logger.connection_refused(&e.to_string());
        }
        self
    }
}

/// An extension trait to convert a `Result<T, E: Into<Response>>` into a `Response`.
pub trait MapResponse<T> {
    #[must_use]
    fn into_response(self, f: impl Fn(T) -> Response) -> Response;
}
impl<T, E: Into<Response>> MapResponse<T> for Result<T, E> {
    /// Turn the `Result` into a `Response`.
    ///
    /// The `Err` variant of the `Result` will be converted to a `Response` using it's `Into`
    /// conversion, the `Ok` variant will call `f` on the contained value.
    fn into_response(self, f: impl Fn(T) -> Response) -> Response {
        self.map_or_else(Into::into, f)
    }
}

/// A trait to be implemented by loggers to log server events.
pub trait Logger: fmt::Debug {
    /// Logs general information about the server such as listening on a port.
    fn info(&self, msg: &str);
    /// Logs that a connection was closed outside of normal circumstance, such as for an invalid key.
    fn connection_refused(&self, msg: &str);
    /// Logs an internal (server) error.
    fn server_error(&self, msg: &str);
}

/// A dummy logger for `server::Server` which does nothing and drops all logs.
#[derive(Debug, Copy, Clone)]
pub struct DummyLogger;
impl DummyLogger {
    /// Make a new `DummyLogger`.
    #[allow(dead_code)]
    #[must_use]
    pub const fn new() -> Self {
        Self {}
    }
}
impl Logger for DummyLogger {
    fn info(&self, _: &str) {}
    fn connection_refused(&self, _: &str) {}
    fn server_error(&self, _: &str) {}
}

pub use private::{Key, Nonce};

/// This module exists to restrict the ability to create `Key` and `Nonce` types. It contains only
/// functions & methods that need to be able to create those types, everything else is placed
/// outside it.
mod private {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use super::{DurationExt, KeyError, NonceError};

    /// A key for a server. A key is an arbitrary string.
    ///
    /// Keys cannot be constructed directly and must be made using an associated function.
    #[non_exhaustive]
    #[derive(Clone, PartialEq, Eq, Hash, serde::Serialize)]
    pub struct Key(pub String);

    // The private impl of `Key`.
    // There's a public one outside of `private`.
    impl Key {
        /// Creates a new `Key` from a string.
        ///
        /// If the string is empty, an error will be returned.
        pub fn new(s: impl Into<String>) -> Result<Self, KeyError> {
            let s = s.into();

            if s.len() != 32 {
                Err(KeyError::WrongSize(s.len()))
            } else if s.chars().any(|c| !(32..127).contains(&(c as u8))) {
                Err(KeyError::InvalidCharacters)
            } else if s.starts_with(' ') || s.ends_with(' ') {
                Err(KeyError::FlankingSpace)
            } else {
                Ok(Self(s))
            }
        }
    }

    /// A server's nonce.
    ///
    /// This can only be constructed
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    pub struct Nonce {
        /// The inner nonce (a unix time stamp).
        inner: u128,
        /// The time into the future that an allowable nonce can be.
        pub leeway: Duration,
    }

    impl Nonce {
        /// Creates a new `Nonce` (set from the current time).
        #[must_use]
        pub fn new(leeway: Duration) -> Self {
            Self {
                inner: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("current time is before unix epoch")
                    .as_millis(),
                leeway,
            }
        }

        /// Begins updating the nonce to a new one, checking for validity.
        ///
        /// A valid nonce is one that came later than this nonce and from no more than `leeway`
        /// amount of time into the future.
        ///
        /// This uses the witness pattern--calling this method will be begin the process of updating
        /// the nonce and give an object which can finalize the update. This allows a separation
        /// between validating the new nonce and updating the value.
        ///
        /// # Example
        ///
        /// ```no_run
        /// use std::time::Duration;
        /// use server::Nonce;
        ///
        /// let mut nonce = Nonce::new(Duration::from_secs(2));
        /// let witness = nonce.begin_update(1337).expect("invalid nonce");
        /// // Check something else here and make sure you really want to update the nonce.
        /// witness.commit()
        /// ```
        pub fn begin_update(
            &mut self,
            new_nonce: u128,
        ) -> Result<NonceValidityWitness, NonceError> {
            if new_nonce <= self.inner {
                Err(NonceError::FromPast)
            } else if new_nonce > (Duration::since_unix_epoch() + self.leeway).as_millis() {
                Err(NonceError::FromFuture)
            } else {
                Ok(NonceValidityWitness(self, new_nonce))
            }
        }
    }

    /// A witness to update a nonce with.
    ///
    /// Upon trying to update a nonce with `Nonce::begin_update`, you'll get a
    /// `NonceValidityWitness` that can be used to actually update the value of the `Nonce` with
    /// it's `commit` method. See `Nonce::begin_update` for more documentation.
    #[must_use = "Call `commit` method to update nonce, see `Nonce::begin_update` for info."]
    #[non_exhaustive]
    #[derive(Debug)]
    pub struct NonceValidityWitness<'a>(&'a mut Nonce, u128);
    impl NonceValidityWitness<'_> {
        /// Finishes updating the `Nonce`.
        ///
        /// See `Nonce::begin_update` for more documentation.
        pub fn commit(self) {
            self.0.inner = self.1;
        }
    }

    #[cfg(test)]
    mod tests {
        use std::time::Duration;

        use super::{
            super::{DurationExt, NonceError},
            Nonce,
        };

        #[test]
        fn test_nonce() {
            let now = Duration::since_unix_epoch().as_millis();
            let mut nonce = Nonce {
                inner: now,
                leeway: Duration::from_secs(2),
            };

            assert_eq!(
                nonce.begin_update(now - 1).unwrap_err(),
                NonceError::FromPast
            );
            assert_eq!(
                nonce.begin_update(now + 5000).unwrap_err(),
                NonceError::FromFuture
            );
            nonce.begin_update(now + 1).unwrap().commit();
            assert_eq!(nonce.inner, now + 1);
        }
    }
}

// The public impl of `Key`.
// There's a private on in `private`;
impl Key {
    /// Generate a secret from this `Key` and a nonce in the form of a base64 string.
    ///
    /// The secret is a hash containing the key and the nonce. It is generated by the client and
    /// sent with the request. The server then generates its own secret using the key and the
    /// nonce the client sent. If the secret the server generates matches the secret the client
    /// sent, then the client has proved it has the key and the request is valid.
    ///
    /// Note: The base64 string is unpadded.
    #[must_use]
    pub(super) fn generate_secret(&self, nonce: u128) -> String {
        let mut hasher = Sha512::new();
        hasher.update(nonce.to_string());
        hasher.update(&self.0);
        hex::encode(hasher.finalize())
    }
}

impl fmt::Debug for Key {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("Key(...)")
    }
}

impl TryFrom<String> for Key {
    type Error = KeyError;

    /// Try to create a `Key` from a string.
    ///
    /// If the string is empty, an error will be returned.
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl<'de> Deserialize<'de> for Key {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = Key;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a valid key")
            }
            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Key::new(v).map_err(serde::de::Error::custom)
            }
        }

        deserializer.deserialize_str(Visitor {})
    }
}

/// Occurs when a key is attempted to be constructed from an empty string.
#[derive(Debug)]
pub enum KeyError {
    /// An error that occurs when the key is the wong size (in bytes).
    WrongSize(usize),
    /// An error that occurs when the key contains non-ascii or non printable characters.
    ///
    /// Note that `\n` and `\t` are not considered printable.
    InvalidCharacters,
    /// An error that occurs when the key begins or ends with space.
    FlankingSpace,
}

impl std::error::Error for KeyError {}
impl fmt::Display for KeyError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use KeyError::*;

        match self {
            WrongSize(size) => write!(f, "key has the wrong size (expected 32 bytes, got {size})"),
            InvalidCharacters => f.write_str(
                "key contains invalid characters. keys can only contain printable ascii characters \
                (no \\n, or \\t)",
            ),
            FlankingSpace => f.write_str("key can't begin or end with a space"),
        }
    }
}

/// An error with a received nonce.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum NonceError {
    /// This occurs when the received nonce is from before the last used nonce.
    FromPast,
    /// This occurs when the received nonce is from further into the future than the `Nonce`'s
    /// `leeway` attribute allows.
    FromFuture,
}

impl From<&NonceError> for &str {
    #[must_use]
    fn from(value: &NonceError) -> Self {
        use NonceError::*;

        match value {
            FromPast => "nonce is too old; are server and client clocks out of sync?",
            FromFuture => {
                "nonce is from too far in the future; are server and client clocks out of sync?"
            }
        }
    }
}

impl Error for NonceError {}
impl fmt::Display for NonceError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(self.into())
    }
}

/// The server.
///
/// Call `Server::run` to run it.
///
/// The server only handles the actual networking and has no knowledge of their content beyond
/// authorization.
#[derive(Debug)]
pub struct Server<L: Logger> {
    /// The socket address to listen on.
    pub addr: SocketAddrV4,
    /// The key to used to validate the connection.
    ///
    /// The key needs to be used by the client to generate the secret used to validate the request.
    pub key: Key,
    /// The logger `Logger` used to log events including general information on connections and
    /// errors.
    ///
    /// If you'd like to ignore log information, use an instance of `DummyLogger`.
    pub logger: L,
}

// The config methods need a concrete type for logger, so we use the dummy logger as a dummy type.
// The logger isn't actually relevant here.
impl Server<DummyLogger> {
    /// Creates a builder for the server config.
    ///
    /// # Example
    ///
    /// ```
    /// use server::{DummyLogger, Key, Server};
    ///
    /// let server = Server::builder()
    ///     .on_localhost()
    ///     .with_port(1337)
    ///     .with_key(Key::new("this is a key and it's 32 bytes.").unwrap())
    ///     .build(DummyLogger::new())
    ///     .expect("missing required config attributes");
    #[must_use]
    #[allow(dead_code)]
    pub fn builder() -> Config {
        Config::new()
    }
}

// TODO: clean this up. the server stuff and the config stuff (do they really need to be separate?)
impl<L: Logger> Server<L> {
    /// Creates a new server from the config specified in the file `config.toml` in the current
    /// directory and with the specified logger.
    ///
    /// # Example
    ///
    /// ```
    /// use server::{DummyLogger, Server};
    ///
    /// let server = Server::from_config_file(DummyLogger::new());
    /// ```
    ///
    /// # config.toml format
    ///
    /// ```text
    /// key = string (32 printable ascii chars, can't begin or end with a space)
    /// address =
    /// port = u16 (0 - 65535)
    /// ```
    #[allow(dead_code)]
    pub fn from_config_file(logger: L) -> Result<Self, ConfigError> {
        Config::new()
            .from_config_file()
            .and_then(|c| Config::build(c, logger))
    }

    /// Run the server.
    ///
    /// This function will only exit if an error occurs.
    pub fn run(&self, f: impl Fn(Request) -> Response) -> Result<(), std::io::Error> {
        let mut buf = vec![0u8; 4096];
        let mut nonce = Nonce::new(Duration::from_secs(2));
        let listener = TcpListener::bind(self.addr)?;

        self.logger.info(&format!("Listening on {}", self.addr));

        for stream in listener.incoming() {
            let Ok(mut stream) = stream.log_connection_refused(&self.logger) else {
                continue;
            };
            if stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .and_then(|()| stream.set_write_timeout(Some(Duration::from_secs(2))))
                .log_connection_refused(&self.logger)
                .is_err()
            {
                let _ = stream
                    .shutdown(Shutdown::Both)
                    .log_connection_refused(&self.logger);
                continue;
            };

            self.validate_connection(&mut stream, &mut buf, &mut nonce)
                .log_connection_refused(&self.logger)
                .ok()
                .flatten()
                .map(|r| f(r).write_to(&mut stream));
            let _ = stream
                .shutdown(Shutdown::Both)
                .log_connection_refused(&self.logger);
        }

        unreachable!();
    }

    /// Receives and validates an incoming connection, returning `Ok(Some(...))` if it's valid and
    /// `Ok(None)` if it isn't.
    fn validate_connection(
        &self,
        stream: &mut TcpStream,
        buf: &mut [u8],
        last_nonce: &mut Nonce,
    ) -> io::Result<Option<Request>> {
        let length = stream.read(buf)?;
        let buf = &buf[..length];

        Ok(match Request::new(buf, &self.key, last_nonce) {
            Err(e) => {
                self.logger.connection_refused(&e.to_string());
                Response::from(&e).write_to(stream)?;
                stream.shutdown(Shutdown::Both)?;
                None
            }
            Ok(request) => {
                self.logger.info(&format!(
                    "Got connection from {} to {}",
                    stream.peer_addr()?,
                    request.path
                ));
                Some(request)
            }
        })
    }
}

/// The config for a `Server`.
///
/// You should usually obtain this through `Server::builder`.
///
/// Trying to build a config without all required attributes set will will result in an error.
/// If an attribute that has a default isn't set, it'll be given it's default value.
///
/// Note that `addr` is called `address` when parsing from a toml file.
///
/// ### Required attributes
///  - `port`
///  - `key`
///
/// ### Default attributes & values
///  - `address` defaults to `0.0.0.0` (lan)
///
/// # Example
///
/// ```
/// use server::{Config, DummyLogger, Key};
///
/// let server = Config::new()
///     .on_localhost()
///     .with_port(1337)
///     .with_key(Key::new("this is a key and it's 32 bytes.").unwrap())
///     .build(DummyLogger::new())
///     .expect("missing required config attributes");
/// ```
#[non_exhaustive]
#[derive(Debug, Default, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct Config {
    /// The ip address to host on.
    ///
    /// Defaults to `0.0.0.0` (lan).
    #[serde(rename = "address")]
    pub addr: Option<Ipv4Addr>,
    /// The port to host on.
    ///
    /// Calling the `build` method will fail if this isn't set.
    pub port: Option<u16>,
    /// The key used to validate the connection.
    ///
    /// Calling the `build` method will fail if this isn't set. See `Key`'s docs for what
    /// constitutes a valid key.
    pub key: Option<Key>,
}

impl Config {
    /// Creates a new `Config`.
    ///
    /// If you're just using the config to build a server, use `Sever::builder` instead.
    pub fn new() -> Self {
        Self::default()
    }

    /// Update this `Config` from a `config.toml` in the current directory.
    ///
    /// Note that the `addr` attribute is called `address` in `config.toml`.
    ///
    /// If an attribute is set on this `Config` and isn't in `config.toml`, it's value will be
    /// maintained. In other words, you can intermix reading config attributes from the file and
    /// setting them in code. Config attributes that have defaults will still use those defaults as
    /// normal if they aren't set in `config.toml` or on the struct.
    ///
    /// ### Required attributes
    ///  - `port`
    ///  - `key`
    ///
    /// ### Default attributes & values
    ///  - `address` defaults to `0.0.0.0` (lan)
    ///
    /// # Example
    ///
    /// ```no_run
    /// // config.toml:
    /// //
    /// // address = "127.0.0.1"
    /// // port = 1337
    /// // key = "this is a key and it's 32 bytes."
    ///
    /// use std::net::{Ipv4Addr, SocketAddrV4};
    /// use server::{Key, DummyLogger, Server};
    ///
    /// let server = Server::builder()
    ///     .from_config_file()
    ///     .expect("failure reading file")
    ///     .build(DummyLogger::new())
    ///     .expect("file didn't contain all necessary items");
    ///
    /// assert_eq!(server.addr, SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 1337));
    /// assert_eq!(server.key, Key::new("this is a key and it's 32 bytes.").unwrap());
    /// ```
    #[allow(dead_code, clippy::wrong_self_convention)]
    pub fn from_config_file(self) -> Result<Self, ConfigError> {
        let mut dir = std::env::current_dir()?;
        dir.push("config.toml");
        self.from_specific_file(dir)
    }

    /// Update this `Config` from a `.toml` file.
    ///
    /// Note that the `addr` attribute is called `address` in the toml file.
    ///
    /// If an attribute is set on this `Config` and isn't in the specified file, it's value will be
    /// maintained. In other words, you can intermix reading config attributes from the file and
    /// setting them in code. Config attributes that have defaults will still use those defaults as
    /// normal if they aren't specified in the file or on the struct.
    ///
    /// ### Required attributes
    ///  - `port`
    ///  - `key`
    ///
    /// ### Default attributes & values
    ///  - `address` defaults to `0.0.0.0` (lan)
    ///
    /// # Example
    ///
    /// ```no_run
    /// // a/b/c/my_config_file.toml:
    /// //
    /// // address = "127.0.0.1"
    /// // port = 1337
    /// // key = "this is a key and it's 32 bytes."
    ///
    /// use std::net::{Ipv4Addr, SocketAddrV4};
    /// use server::{DummyLogger, Key, Server};
    ///
    /// let server = Server::builder()
    ///     .from_specific_file("a/b/c/my_config_file.toml")
    ///     .expect("failure reading file")
    ///     .build(DummyLogger::new())
    ///     .expect("file didn't contain all necessary items");
    ///
    /// assert_eq!(server.addr, SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 1337));
    /// assert_eq!(server.key, Key::new("this is a key and it's 32 bytes.").unwrap());
    /// ```
    #[allow(clippy::wrong_self_convention)]
    pub fn from_specific_file(
        mut self,
        config_file: impl AsRef<Path>,
    ) -> Result<Self, ConfigError> {
        let config_file = config_file.as_ref();

        let file_content = match std::fs::read_to_string(config_file) {
            Ok(i) => i,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Err(ConfigError::FileNotFound(config_file.to_owned()))
            }
            Err(e) => return Err(ConfigError::Io(e)),
        };
        let new: Self = toml::from_str(&file_content)?;

        self.addr = new.addr.or(self.addr);
        self.port = new.port.or(self.port);
        self.key = new.key.or(self.key);

        Ok(self)
    }

    /// Gets the socket address of this `Config`.
    ///
    /// # Example
    ///
    /// ```
    /// use std::net::{Ipv4Addr, SocketAddrV4};
    /// use server::Server;
    ///
    /// let ip = Ipv4Addr::new(127, 0, 0, 1);
    /// let config = Server::builder().with_addr(ip).with_port(1337);
    ///
    /// assert_eq!(config.sock_addr(), Some(SocketAddrV4::new(ip, 1337)));
    /// ```
    #[must_use]
    #[allow(dead_code)]
    pub fn sock_addr(&self) -> Option<SocketAddrV4> {
        self.addr
            .zip(self.port)
            .map(|(addr, port)| SocketAddrV4::new(addr, port))
    }

    /// Build this `Config` into a `Server`.
    ///
    /// Any unset attributes will be given their default values if they have them, otherwise an
    /// `Err` will be returned (for required attributes).
    ///
    /// ### Required attributes
    ///  - `port`
    ///  - `key`
    ///
    /// ### Default attributes & values
    ///  - `address` defaults to `0.0.0.0`
    ///
    /// # Example
    ///
    /// ```
    /// use server::{DummyLogger, Key, Server};
    ///
    /// let server = Server::builder()
    ///     .on_localhost()
    ///     .with_port(1337)
    ///     .with_key(Key::new("this is a key and it's 32 bytes.").unwrap())
    ///     .build(DummyLogger::new())
    ///     .expect("missing required config attributes");
    /// ```
    pub fn build<L: Logger>(self, logger: L) -> Result<Server<L>, ConfigError> {
        let key = self.key.ok_or(ConfigError::MissingRequired("key"))?;
        let addr = self.addr.unwrap_or_else(|| Ipv4Addr::new(0, 0, 0, 0));
        let port = self.port.ok_or(ConfigError::MissingRequired("port"))?;

        Ok(Server {
            addr: SocketAddrV4::new(addr, port),
            key,
            logger,
        })
    }

    /// Sets the ip address of this `Config`.
    ///
    /// # Example
    ///
    /// ```
    /// use std::net::Ipv4Addr;
    /// use server::Server;
    ///
    /// let addr = Ipv4Addr::new(127, 0, 0, 1);
    /// let config = Server::builder().with_addr(addr);
    ///
    /// assert_eq!(config.addr, Some(addr));
    /// ```
    #[allow(dead_code)]
    pub fn with_addr(mut self, addr: Ipv4Addr) -> Self {
        self.addr = Some(addr);
        self
    }
    /// Sets the ip address of this `Config` to localhost (`127.0.0.1`).
    ///
    /// # Example
    ///
    /// ```
    /// use std::net::Ipv4Addr;
    /// use server::Server;
    ///
    /// let config = Server::builder().on_localhost();
    ///
    /// assert_eq!(config.addr, Some(Ipv4Addr::new(127, 0, 0, 1)));
    /// ```
    #[allow(dead_code)]
    pub fn on_localhost(self) -> Self {
        self.with_addr(Ipv4Addr::new(127, 0, 0, 1))
    }
    /// Sets the ip address of this `Config` to lan (`0.0.0.0`).
    ///
    /// This means the server will be accessible on your local network.
    ///
    /// # Example
    ///
    /// ```
    /// use std::net::Ipv4Addr;
    /// use server::Server;
    ///
    ///
    /// let config = Server::builder().on_lan();
    ///
    /// assert_eq!(config.addr, Some(Ipv4Addr::new(0, 0, 0, 0)));
    /// ```
    #[allow(dead_code)]
    pub fn on_lan(self) -> Self {
        self.with_addr(Ipv4Addr::new(0, 0, 0, 0))
    }
    /// Sets the port of this `Config`.
    ///
    /// # Example
    ///
    /// ```
    /// use server::Server;
    ///
    /// let config = Server::builder().with_port(1337);
    ///
    /// assert_eq!(config.port, Some(1337));
    /// ```
    #[allow(dead_code)]
    pub fn with_port(mut self, port: u16) -> Self {
        self.port = Some(port);
        self
    }
    /// Sets the socket address of this `Config`.
    ///
    /// # Example
    ///
    /// ```
    /// use std::net::{Ipv4Addr, SocketAddrV4};
    /// use server::Server;
    ///
    /// let sock_addr = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 1337);
    /// let config = Server::builder().with_sock_addr(sock_addr);
    ///
    /// assert_eq!(config.sock_addr(), Some(sock_addr));
    /// ```
    #[allow(dead_code)]
    pub fn with_sock_addr(mut self, sock_addr: SocketAddrV4) -> Self {
        self.addr = Some(*sock_addr.ip());
        self.port = Some(sock_addr.port());
        self
    }
    /// Sets the key of this `Config`.
    ///
    /// # Example
    ///
    /// ```
    /// use server::{Key, Server};
    ///
    /// let key = Key::new("this is a key and it's 32 bytes.").unwrap();
    /// let config = Server::builder().with_key(key.clone());
    ///
    /// assert_eq!(config.key, Some(key));
    /// ```
    #[allow(dead_code)]
    pub fn with_key(mut self, key: Key) -> Self {
        self.key = Some(key);
        self
    }
}

/// Returned when there is an error with parsing or building a `Config`.
///
/// This happens when:
///   - a config without required attributes is built into a server
///   - an io error occurred when opening or reading a config file
///   - a config file contains invalid toml or invalid types/data
#[derive(Debug)]
pub enum ConfigError {
    /// Returned when an attribute is missing form the `Config`.
    ///
    /// See `Config`'s documentation for required attributes.
    MissingRequired(&'static str),
    /// Returned when opening and reading a config file encounters an error.
    Io(io::Error),
    /// Returned when the config file is not found.
    FileNotFound(PathBuf),
    /// Returned when a toml error is encountered when paring a config file.
    ///
    /// Invalid addresses, ports, keys, etc... in a config file are considered toml errors and
    /// wrapped in this type.
    Toml(toml::de::Error),
}

impl Error for ConfigError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::MissingRequired(_) => None,
            Self::Io(e) => Some(e),
            Self::FileNotFound(_) => None,
            Self::Toml(e) => Some(e),
        }
    }
}
impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::MissingRequired(field) => write!(f, "no {field} set"),
            Self::FileNotFound(path) => write!(f, "no config file found at {}", path.display()),
            _ => fmt::Display::fmt(&self.source().unwrap(), f),
        }
    }
}

impl From<io::Error> for ConfigError {
    fn from(err: io::Error) -> Self {
        #[cfg(debug_assertions)]
        if err.kind() == io::ErrorKind::NotFound {
            panic!(
                "use `ConfigError::FileNotFound` for missing config file, not `ConfigError::Io`"
            );
        }

        Self::Io(err)
    }
}
impl From<toml::de::Error> for ConfigError {
    fn from(err: toml::de::Error) -> Self {
        Self::Toml(err)
    }
}

/// An http method.
///
/// Currently only get nad post are supported as those are the only methods used.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Method {
    /// The GET http method.
    Get,
    /// The POST http method.
    Post,
}

impl TryFrom<&str> for Method {
    type Error = RequestError;

    /// Try to create a `Method` from a `&str`.
    ///
    /// If the the string isn't one of `GET` or `POST`, an `Err(RequestError::InvalidHttp)` error is
    /// returned.
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "GET" => Ok(Self::Get),
            "POST" => Ok(Self::Post),
            _ => Err(RequestError::MalformedHttp),
        }
    }
}
impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(match self {
            Self::Get => "GET",
            Self::Post => "POST",
        })
    }
}

/// An http request.
///
/// A `Request` instance can only be constructed with associated functions from a valid http
/// request--this means that for a `Request` instance to exist, it necessarily had the proper key
/// and a proper nonce (unless constructed directly).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// The http method of the request.
    pub method: Method,
    /// The path the request was made to, including the slash.
    pub path: String,
}

impl Request {
    /// Creates a new `Request`.
    ///
    /// The key and last nonce are required to validate the request. As all requests must be
    /// validated, any `Request` instance that exists is inherently a valid request.
    fn new(buf: &[u8], key: &Key, last_nonce: &mut Nonce) -> Result<Self, RequestError> {
        use RequestError::*;

        // we take until the end of the headers (a blank line)
        let buf_as_str = String::from_utf8_lossy(buf);
        let mut lines = buf_as_str.lines().take_while(|line| !line.is_empty());

        let mut line1 = lines.next().ok_or(MalformedHttp)?.splitn(3, ' ');
        let method = line1.next().ok_or(MalformedHttp)?.try_into()?;
        let path = line1.next().ok_or(MalformedHttp)?;

        if path == "/favicon.ico" {
            return Err(IllegalEndpoint(Cow::Borrowed("/favicon.ico")));
        }

        // Get the headers
        let lines: Vec<_> = lines.filter_map(|i| i.split_once(": ")).collect();
        let secret = lines
            .iter()
            .find(|(k, _)| *k == "Secret")
            .ok_or(MissingSecret)?
            .1;
        let nonce: u128 = lines
            .iter()
            .find(|(k, _)| *k == "Nonce")
            .ok_or(MissingNonce)?
            .1
            .parse()
            .map_err(|_| RequestError::MalformedHeaders)?;

        let nonce_witness = last_nonce.begin_update(nonce)?;
        if key.generate_secret(nonce) == secret {
            nonce_witness.commit();
            Ok(Self {
                method,
                path: path.to_owned(),
            })
        } else {
            Err(RequestError::InvalidKey)
        }
    }
}

/// Occurs during the creation of a `Request`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestError {
    /// Occurs when the http is malformed.
    MalformedHttp,
    /// Occurs when a header is malformed.
    MalformedHeaders,
    /// Occurs when the nonce header is missing.
    MissingNonce,
    /// Occurs when the secret header is missing.
    MissingSecret,
    /// Occurs when a request is made to an illegal endpoint.
    IllegalEndpoint(Cow<'static, str>),
    /// Occurs when the key is invalid (because the secret doesn't match).
    InvalidKey,
    /// Occurs when the nonce is invalid.
    NonceError(NonceError),
}

impl Error for RequestError {}
impl fmt::Display for RequestError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use Cow::{Borrowed, Owned};
        use RequestError::*;

        f.write_str(&match self {
            MalformedHttp => Borrowed("http is malformed"),
            MalformedHeaders => Borrowed("a header is malformed"),
            MissingNonce => Borrowed("nonce header not found"),
            MissingSecret => Borrowed("secret header not found"),
            IllegalEndpoint(i) => Owned(format!("tried to reach illegal endpoint {i}")),
            InvalidKey => Borrowed("key is invalid"),
            NonceError(e) => Owned(e.to_string()),
        })
    }
}

impl From<NonceError> for RequestError {
    /// Creates a `RequestError::NonceError` from a `NonceError`.
    #[must_use]
    fn from(value: NonceError) -> Self {
        Self::NonceError(value)
    }
}

impl From<&RequestError> for Response {
    /// Creates an error `Response` from a `RequestError` that indicated the error.
    #[must_use]
    fn from(value: &RequestError) -> Self {
        let status = match value {
            RequestError::InvalidKey => 401,
            _ => 400,
        };
        Self::from_message(status, value.to_string())
    }
}

/// The content of a response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponseContent {
    /// For when a response has no content.
    None,
    /// For when a response has text content.
    Text(String),
    /// For when a response's content is a png.
    Png(Vec<u8>),
}

impl ResponseContent {
    /// Converts the content into a byte vector.
    ///
    /// If there is no content (the `None` variant), the vector will be empty.
    #[must_use]
    fn as_bytes(&self) -> &[u8] {
        use ResponseContent::*;

        match self {
            None => &[],
            Text(s) => s.as_bytes(),
            Png(b) => b,
        }
    }

    /// The http-formatted 'Content-Type' header, with a trailing newline.
    ///
    /// If the content is the `None` variant, the string will be empty with no newline.
    #[must_use]
    fn content_type_header_repr(&self) -> &str {
        use ResponseContent::*;

        match self {
            None => "",
            Text(_) => "Content-Type: text/plain; charset=utf-8\r\n",
            Png(_) => "Content-Type: image/png\r\n",
        }
    }

    /// The length of the content in bytes.
    ///
    /// This is equivalent to `ResponseContent::into_bytes().len()`.
    #[must_use]
    fn len(&self) -> usize {
        use ResponseContent::*;

        match self {
            None => 0,
            Text(s) => s.len(),
            Png(b) => b.len(),
        }
    }
}

/// An http response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    /// The http status of the response.
    pub status: u16,
    /// The content of the response.
    pub content: ResponseContent,
}

impl Response {
    /// Creates a new `Response` with a status code (and no content).
    #[must_use]
    pub fn from_status(status: u16) -> Self {
        Self {
            status,
            content: ResponseContent::None,
        }
    }

    /// Creates a new `Response` with a status code and the content of `msg`.
    #[must_use]
    pub fn from_message(status: u16, msg: String) -> Self {
        Self {
            status,
            content: ResponseContent::Text(msg),
        }
    }

    /// Creates a new `Response` with a status code and content of a png.
    #[must_use]
    pub fn from_png(png: Vec<u8>) -> Self {
        Self {
            status: 200,
            content: ResponseContent::Png(png),
        }
    }

    /// Generates the http headers of this response (including ending blank line).
    #[must_use]
    fn generate_headers(&self) -> String {
        format!(
            "HTTP/1.1 {}\r\n{}Content-Length: \
             {}\r\n\r\n",
            self.status,
            self.content.content_type_header_repr(),
            self.content.len(),
        )
    }

    /// Writes the http of this response to a `TcpStream`.
    fn write_to(&self, stream: &mut TcpStream) -> std::io::Result<()> {
        stream.write_all(self.generate_headers().as_bytes())?;
        stream.write_all(self.content.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    /// Generates the http of what a response should look like from status and content.
    fn format_http_response(status: u16, content: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\n{}Content-Length: {}\r\n\r\n{content}",
            if content.is_empty() {
                ""
            } else {
                "Content-Type: text/plain; charset=utf-8\r\n"
            },
            content.len(),
        )
    }

    #[test]
    fn test_server() {
        use std::io::{Read, Write};
        use std::net::SocketAddrV4;
        use std::thread;
        use std::time::Duration;

        use super::{
            Config, DummyLogger, DurationExt, Key, Method, Request, RequestError, Response,
        };

        struct ClientMock {
            dst_addr: SocketAddrV4,
            key: Key,
            buf: Vec<u8>,
        }

        impl ClientMock {
            fn from_config(config: Config) -> Self {
                Self {
                    dst_addr: config.sock_addr().unwrap(),
                    key: config.key.unwrap(),
                    buf: vec![0; 4096],
                }
            }

            fn send_request(&mut self, method: Method, path: &str) -> String {
                // We sleep to make sure the nonce is different.
                thread::sleep(Duration::from_millis(1));
                let mut stream = std::net::TcpStream::connect(self.dst_addr).unwrap();
                let nonce = Duration::since_unix_epoch().as_millis();
                let http = format!(
                    "{method} {path} HTTP/1.1\r\nContent-Length: 0\r\nNonce: {nonce}\r\nSecret: \
                     {}\r\n\r\n",
                    self.key.generate_secret(nonce)
                );

                stream.write_all(http.as_bytes()).unwrap();
                // The server writes the headers and content with two separate `write` calls, and if
                // we read immediately without waiting first, we'll only get the first `write` call.
                // TODO: should probably make this less of a horrible hack
                thread::sleep(Duration::from_millis(100));
                let len = stream.read(&mut self.buf).unwrap();
                String::from_utf8_lossy(&self.buf[0..len]).to_string()
            }
        }

        // We can't use a random port (port 0) because it gets chosen when bound, at which point
        // it's opaque in the server's `run` method. So we just use this port instead. Call it
        // compile-time random. https://xkcd.com/221.
        let config = Config::new()
            .on_localhost()
            .with_port(35621)
            .with_key(Key::new("this is a key and it's 32 bytes.").expect("invalid (real) key"));
        let fake_key = Key::new("this is a fake key, valid though").expect("invalid (fake) key");

        let mut client = ClientMock::from_config(config.clone());
        let server = config
            .clone()
            .build(DummyLogger::new())
            .expect("failed to build server from config");

        let server_res = std::thread::spawn(move || {
            server
                .run(|r| match r {
                    Request {
                        method: Method::Get,
                        path,
                    } if path == "/test1" => Response::from_status(200),
                    Request {
                        method: Method::Post,
                        path,
                    } if path == "/test2" => {
                        Response::from_message(400, "this is a message".to_owned())
                    }
                    Request { method, path } => panic!("received {method} request to {path}"),
                })
                .unwrap();
        });

        assert_eq!(
            client.send_request(Method::Get, "/test1"),
            format_http_response(200, "")
        );
        assert_eq!(
            client.send_request(Method::Post, "/test2"),
            format_http_response(400, "this is a message")
        );

        // Test invalid key
        {
            client.key = fake_key;
            assert_eq!(
                client.send_request(Method::Get, "/invalid_endpoint"),
                format_http_response(401, &RequestError::InvalidKey.to_string())
            );
            client.key = config.key.unwrap();
        }

        // Stop the server by sending it a request to an unexpected endpoint, causing it to panic.
        client.send_request(Method::Get, "/stop");

        // Pull the value out of the threads panic and ignore it if it's our stop request, otherwise
        // reraise it.
        if let Err(e) = server_res.join() {
            match e.downcast::<String>() {
                Ok(s) if *s == "received GET request to /stop" => (),
                i => panic!("{i:?}"),
            }
        }
    }
}
