#![windows_subsystem = "windows"]
#![feature(decl_macro, proc_macro_hygiene)]

use std::cmp;
use std::convert::TryFrom;
use std::fmt;
use std::io::Cursor;
use std::str::FromStr;
use std::string::ToString;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use lazy_static::lazy_static;
use rocket::request::Outcome;
use rocket::{get, http::ContentType, http::Status, request, response::Response, routes};
use sha2::{Digest, Sha512};

mod util;

const ALLOWABLE_TIME_DIFFERENCE: Duration = Duration::from_secs(2);
static LAST_USED_NONCE: AtomicU64 = AtomicU64::new(0);
lazy_static! {
    static ref KEY: String = dotenv::var("KEY").expect("No key is set in `.env`");
}

#[derive(Debug, Copy, Clone)]
enum Error {
    Missing,
    Invalid,
    NonceFromPast,
    NonceFromFuture,
}

impl std::error::Error for Error {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use Error::*;

        match self {
            Missing => write!(f, "nonce and/or secret headers are missing"),
            Invalid => write!(f, "secret and/or nonce are invalid"),
            NonceFromPast => write!(f, "newer nonce has already been used"),
            NonceFromFuture => write!(f, "nonce is from future"),
        }
    }
}

impl From<Error> for Outcome<Secret, Error> {
    fn from(error: Error) -> Self {
        use Error::*;

        match error {
            Missing | Invalid => Outcome::Failure((Status::BadRequest, error)),
            NonceFromPast | NonceFromFuture => Outcome::Failure((Status::NotAcceptable, error)),
        }
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
struct Nonce(u64);

impl Nonce {
    /// Initializes the last used nonce to the current time delta. Nonces before this time will be
    /// invalid.
    fn initialize_last_used() {
        LAST_USED_NONCE.store(time_since_epoch(), Ordering::SeqCst);
    }

    /// Gets the last used nonce.
    fn get_last_used() -> Self {
        Self::new(LAST_USED_NONCE.load(Ordering::SeqCst))
    }

    /// Sets the last used nonce.
    fn set_last_used(self) {
        LAST_USED_NONCE.store(*self.inner(), Ordering::SeqCst);
    }

    /// Creates a new `Nonce` from a `u64`.
    const fn new(n: u64) -> Self {
        Self(n)
    }

    /// Gets the inner `u64` of this `Nonce`.
    fn inner(&self) -> &u64 {
        &self.0
    }

    /// Returns a `Result` that contains an error if any apply to this `Nonce`.
    fn validity(self) -> Result<(), Error> {
        if self <= Nonce::get_last_used() {
            Err(Error::NonceFromPast)
        } else if self > time_since_epoch() {
            Err(Error::NonceFromFuture)
        } else {
            Ok(())
        }
    }

    /// Generates a secret using this `Nonce` and a `key`.
    fn generate_secret(self, key: impl AsRef<[u8]>) -> String {
        let mut hasher = Sha512::new();
        hasher.input(self.inner().to_string());
        hasher.input(key);
        hex::encode(hasher.result())
    }
}

impl TryFrom<&request::Request<'_>> for Nonce {
    type Error = Error;
    fn try_from(r: &request::Request) -> Result<Self, Self::Error> {
        match r.headers().get_one("Nonce") {
            Some(n) => Nonce::from_str(n),
            None => Err(Error::Missing),
        }
    }
}

impl FromStr for Nonce {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Error> {
        s.parse::<u64>().map(Self).map_err(|_| Error::Invalid)
    }
}

impl ToString for Nonce {
    fn to_string(&self) -> String {
        self.0.to_string()
    }
}

impl PartialEq<u64> for Nonce {
    fn eq(&self, rhs: &u64) -> bool {
        self.inner() == rhs
    }
}

impl PartialOrd<u64> for Nonce {
    fn partial_cmp(&self, rhs: &u64) -> Option<cmp::Ordering> {
        Some(self.inner().cmp(rhs))
    }
}

/// This is used as a request guard and will fail if the secret (or nonce) is invalid.
struct Secret;

impl Secret {
    fn new() -> Self {
        Self {}
    }
}

impl<'a, 'r> request::FromRequest<'a, 'r> for Secret {
    type Error = Error;

    fn from_request(r: &'a request::Request<'r>) -> Outcome<Self, Self::Error> {
        let user_secret = match r.headers().get_one("Secret") {
            Some(s) => s,
            None => return Self::Error::Missing.into(),
        };
        let nonce = match Nonce::try_from(r) {
            Ok(i) => i,
            Err(e) => return e.into(),
        };

        if let Err(e) = nonce.validity() {
            return e.into();
        }
        if user_secret != nonce.generate_secret(&*KEY) {
            return Self::Error::Invalid.into();
        }

        nonce.set_last_used();

        Outcome::Success(Secret::new())
    }
}

#[get("/ping")]
fn ping(_secret: Secret) -> Status {
    Status::Ok
}

#[get("/sleep")]
fn sleep(_secret: Secret) -> Status {
    // Always returns Status::Accepted, even if it failed.
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(10));
        util::sleep_computer();
    });

    Status::Accepted
}

#[get("/sleep_display")]
fn sleep_display(_secret: Secret) -> Status {
    util::sleep_display();

    Status::Accepted
}

#[get("/minimize")]
fn minimize(_secret: Secret) -> Status {
    match util::minimize_windows() {
        Ok(_) => Status::Accepted,
        Err(_) => Status::InternalServerError,
    }
}

#[get("/screenshot")]
fn screenshot<'a>(_secret: Secret) -> Response<'a> {
    Response::build()
        .header(ContentType::PNG)
        .sized_body(Cursor::new(util::take_screenshot()))
        .finalize()
}

/// Return the time, in seconds, since the epoch (the Unix epoch is used).
fn time_since_epoch() -> u64 {
    (SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0))
        + ALLOWABLE_TIME_DIFFERENCE)
        .as_secs()
}

fn main() {
    Nonce::initialize_last_used();

    rocket::ignite()
        .mount("/", routes![sleep, sleep_display, minimize, screenshot])
        .launch();
}
