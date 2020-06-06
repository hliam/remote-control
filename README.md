# Remote-control

Remote-control is a [Rocket](https://rocket.rs) server that allows basic functions to be performed
on a computer remotely from the Shortcuts \*OS app. Licensed under [MIT license](./LICENSE).


## Table of Contents & Quick Links <a name = "table-of-contents"></a>
- [About](#about)
- [Features](#features)
- [Getting Started](#getting-started) [[computer](#getting-started-computer)] [[iDevice](#getting-started-idevice)]
- [Deploy (& Build)](#deploy) [[computer](#deploy-computer)] [[iDevice](#deploy-idevice)]
- [Using in Shortcuts](#using-in-shortcuts)
- [Example Setups](#example-setups)
- [Uninstall](#uninstall)
- [Disclaimer](#disclaimer)
- Quick links: [[master shortcut](https://www.icloud.com/shortcuts/761eb83ac4e84b479f4e016ea4e702aa)]
               [[computer-specific shortcut](https://www.icloud.com/shortcuts/6140e36672464b69a6c5fbea1621b785)]


## About <a name = "about"></a>
Remote-control is primary intended for use with the Shortcuts \*OS app (and may be
limited in various ways to maintain easy Shortcuts compatibility). Currently, this is only
compatible with Windows for the time being. You will also have to make DHCP reservations for the
computers that you wish to remotely control.

Note that the screenshot functionality is not encrypted and should only be used on secure, trusted
networks. This is due to the limitations of Shortcuts.


## Features <a name = "features"></a>
- Put computer to sleep
- Put display to sleep
- Minimize all open windows
- Take a screenshot (of the primary monitor)
- Ping to see if computer is awake and connected to internet


## Getting Started <a name = "getting"></a>
To setup Remote-control, we'll need to put files on the computer(s) we want to control as well as
shortcuts on the iDevice(s) we want to control them from.


### Computer <a name = "getting-started-computer"></a>
First, we'll need to setup the server on the computer we want to remotely control.

You'll need to install the nightly channel unless you already have it installed.

```bash
rustup toolchain install nightly
```

We set nightly to be the default (only for this project)
```bash
rustup override set nightly
```

Clone the repository
```bash
git clone https://github.com/hliam/remote-control
```

Install the Python requirments
```bash
python3 -m pip install -r requirements.txt
```

We'll need to create a file called `.env` in that directory that contains the text `KEY={yourKey}`
where '{yourKey}' is the verification key we want to use. This key can be anything we want.

Then we'll need to configure the server. We could configure this through environment variables in
the `.env` file if we would like, but we'll be using `Rocket.toml` for the configuration. Note
that setting the address to `0.0.0.0` will make the server available on your local network.

Create a file in the directory called `Rocket.toml` with the following text:
```toml
[global]
address = "0.0.0.0"
port = {yourPort}
workers = 1

[development]
log = "debug"
```
Where `{yourPort}` is the port you want to host on.

Now that everything is set up on the computer, we'll need to make a DHCP reservation for it (only
follow this if you have DHCP enabled (as opposed to static ips)--if you don't know if you have DHCP
enabled, you probably do). This will ensure that the local ip of the computer on the network will
never change. How to do this varies depending on router you're using, but you can probably find the
network configuration by typing `192.168.0.1` into the address bar of a browser. Then you'll need to
find where it shows connected devices, find the computer you want to use, and create a DHCP
reservation for it. Note that only the computer(s) you want to control need a DHCP reservation, not
your iDevice.

We should now be ready to deploy the server, check out the [deploy](#deploy) section to see how.


### iDevice <a name = "getting-started-idevice"></a>
While we can interface with Remote-control any way we want, it's intended to be used with the
Shortcuts \*OS app.

First, we need the master shortcut. This shortcut will provide an api to connect to the server from
other shortcuts; it is not intended to be run directly. You should only ever have one of this
shortcut in your Shortcuts library. Get it
[here](https://www.icloud.com/shortcuts/761eb83ac4e84b479f4e016ea4e702aa), then save it to your
shortcuts library.

Now that the scaffolding is setup, we can move on to deploying to our computer(s) and iDevice.
Check out the [deploy](#deploy) section to see how.


## Deploy (& Build) <a name = "deploy"></a>
Once all the [setup](#getting-started) is done, we're ready to deploy!


### Computer <a name = "deploy-computer"></a>
To deploy the server, just use the deploy script
(TODO)
```bash
python ./deploy.py
```

This will build and deploy the server (and set Rocket's environment to 'production'). The server
will now be running and will also start automatically when the computer is started And that's it!
The server is now ready to receive requests! (TODO: put some more documentation about this is deploy script)


### iDevice <a name = "deploy-idevice"></a>
For each computer we want to control, we'll need a copy of
[this shortcut](https://www.icloud.com/shortcuts/6140e36672464b69a6c5fbea1621b785) (which we'll
refer to as the computer-specific shortcut). Upon importing it, you'll be asked for the ip and port
of the computer and the key used. We'll also want to put the name of the computer it's connecting
to in the title. The placeholder for this name is '{computerName}'--it should be changed to
something more descriptive, like 'laptop'. Alright, now we have everything set up and we can finally
start making shortcuts! Checkout the [using in Shortcuts](#using-in-shortcuts) section to see how.
You can also checkout the [example setups](#example-setups) section for some examples.


## Using in Shortcuts <a name = "using-in-shortcuts"></a>
Once the server is up and the necessary shortcuts are in your shortcuts library, we can start making
requests to the server. To do this, we'll call the computer-specific shortcut of the computer we
want to control and set it's input to some text. The text we give it is a path on the server,
```
https://google.com/search/for/something
                  ^^^^^^^^^^^^^^^^^^^^^
                  this part is the path
```
so it will always start with a `/` and be followed by the action you want to perform. Available
actions are:
- `sleep` - put computer to sleep
- `sleep_display` - turn display off
- `minimize_windows` - minimize all open windows
- `screenshot` - take a screenshot (of the primary display)
- `ping` - ping the computer to see if it's awake, connected to internet, and the server's running

For example, if we wanted to sleep the display, we would run the `_connectTo{computerName}Shortcut`
shortcut and set its input to `/sleep_display`.


## Example Setups <a name = "example-setups"></a>
TODO
- Ping shortcut
- Menu with all actions


## Uninstall
To uninstall, run the deploy script with the `--uninstall` flag (TODO: add this; kill it and remove startup stuff)
```bash
python ./deploy.py --uninstall
```
