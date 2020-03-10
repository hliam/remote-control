This project will host a [Rocket](https://rocket.rs) server on a computer and allow basic functions to be
accessed through the Shortcuts iOS app.

[This](https://www.icloud.com/shortcuts/17e6def282af45b79294952d8823af8f) shortcut is required to interface with any computer running the server. [This](https://www.icloud.com/shortcuts/c1a22e3eec4842218800de2146353b43)
shortcut specializes the previous shortcut to interface with one particular computer. Relevant information
such as the key and url will asked when the latter shortcut is imported. A `.env` file must be created
and must have a variable called `KEY`. A `Rocket.toml` must also be created with config information,
including the port number and address to host at.

Note that this is in no way secure and should not be considered as such. It uses a key that is hashed
(once) with SHA512 and sent with a nonce--this is meant as a temporary deterrence, not as security.
Do not use this on untrusted networks.

Note also that this is prone to breaking during time changes as it uses the time delta as a nonce. This
may change in the future, but such functionality is heavily limited by the limitations of the Shortcuts
app.
