use std::borrow::Cow;
use std::convert::{TryFrom, TryInto};
use std::error::Error;
use std::fmt;
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, Shutdown, SocketAddrV4, TcpListener, TcpStream};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha512};

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

    use super::{DurationExt, EmptyKeyError, NonceError};

    /// A key for a server. A key is an arbitrary string.
    ///
    /// Keys cannot be constructed directly and must be made using an associated function.
    #[non_exhaustive]
    #[derive(Clone, PartialEq, Eq)]
    pub struct Key(pub String);

    impl Key {
        /// Creates a new `Key` from a string.
        ///
        /// If the string is empty, an error will be returned.
        pub fn new(s: impl Into<String>) -> Result<Self, EmptyKeyError> {
            let s = s.into();

            if s.is_empty() {
                Err(EmptyKeyError)
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
        /// ```
        /// let nonce = Nonce::new();
        /// let witness = nonce.begin_update(1337).expect("invalid nonce");
        /// // Check something else here and make sure you really want to update the nonce.
        /// witness.finalize_update()
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
    /// `NonceValidityWitness` that can be used to finishing updating the value of the `Nonce` with
    /// it's `finalize_update` method. See `Nonce::begin_update` for more documentation.
    #[must_use = "Call `finalize_update` method to update nonce, see `Nonce::begin_update` for info."]
    #[non_exhaustive]
    #[derive(Debug)]
    pub struct NonceValidityWitness<'a>(&'a mut Nonce, u128);
    impl NonceValidityWitness<'_> {
        /// Finishes updating the `Nonce`.
        ///
        /// See `Nonce::begin_update` for more documentation.
        pub fn finalize_update(self) {
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
            nonce.begin_update(now + 1).unwrap().finalize_update();
            assert_eq!(nonce.inner, now + 1);
        }
    }
}

impl Key {
    /// Creates a new `Key` from the environment variable `REMOTE_CONTROL_KEY` in a .env file.
    ///
    /// This function will panic if the environment variable can't be found. If the string is
    /// empty, an error will be returned.
    pub fn from_env(env_var_name: &str) -> Result<Self, EmptyKeyError> {
        dotenvy::var(env_var_name)
            .expect("no key found in .env file")
            .try_into()
    }

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
    type Error = EmptyKeyError;

    /// Try to create a `Key` from a string.
    ///
    /// If the string is empty, an error will be returned.
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

/// Occurs when a key is attempted to be constructed from an empty string.
#[derive(Debug)]
pub struct EmptyKeyError;

impl std::error::Error for EmptyKeyError {}
impl fmt::Display for EmptyKeyError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("key is empty")
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

impl<L: Logger> Server<L> {
    /// Creates a new `Server` that listens on 0.0.0.0 (LAN).
    ///
    /// If you'd like to listen on an address other than 0.0.0.0, you can use `Server::from_addr`.
    /// If you'd like to ignore log information, use an instance of `DummyLogger` as the logger.
    #[must_use]
    pub fn new(port: u16, key: Key, logger: L) -> Self {
        Self::from_addr(
            SocketAddrV4::new(Ipv4Addr::new(0, 0, 0, 0), port),
            key,
            logger,
        )
    }

    /// Creates a new `Server`.
    ///
    /// If you'd like to ignore log information, use an instance of `DummyLogger` as the logger.
    #[must_use]
    pub fn from_addr(addr: SocketAddrV4, key: Key, logger: L) -> Self {
        Self { addr, key, logger }
    }

    /// Creates a new `Server` with a key and port from a .env file.
    /// If you'd like to ignore log information, use an instance of `DummyLogger` as the logger.
    pub fn from_env(
        key_env_var: &str,
        port_env_var: &str,
        logger: L,
    ) -> Result<Self, EmptyKeyError> {
        Key::from_env(key_env_var).map(|key| {
            let port = dotenvy::var(port_env_var)
                .expect("port not found in .env file (set `REMOTE_CONTROL_PORT`)")
                .parse()
                .expect("port (set from .env) isn't valid; ports must be less than 65535");
            Self::new(port, key, logger)
        })
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

        panic!();
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
        let lines: Vec<_> = lines.flat_map(|i| i.split_once(": ")).collect();
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
            nonce_witness.finalize_update();
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
        use std::net::{Ipv4Addr, SocketAddrV4};
        use std::thread;
        use std::time::Duration;

        use super::{
            DummyLogger, DurationExt, Key, Method, Request, RequestError, Response, Server,
        };

        struct ClientMock {
            dst_addr: SocketAddrV4,
            key: Key,
            buf: Vec<u8>,
        }

        impl ClientMock {
            fn new(dst_addr: SocketAddrV4, key: Key) -> Self {
                Self {
                    dst_addr,
                    key,
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
        let server_addr = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 35621);
        let key = Key::new("this is a key").unwrap();
        let mut client = ClientMock::new(server_addr, key.clone());
        let server = Server::from_addr(server_addr, key.clone(), DummyLogger::new());

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

        // We switch to an invalid key.
        client.key = Key::new("a different key").unwrap();

        assert_eq!(
            client.send_request(Method::Get, "/invalid_endpoint"),
            format_http_response(401, &RequestError::InvalidKey.to_string())
        );

        // Switch back to valid key.
        client.key = key;

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
