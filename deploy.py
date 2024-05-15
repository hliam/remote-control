# To use this, you must have a `.env` file that contains a `KEY` environment variable.

import os
from contextlib import redirect_stdout
from dataclasses import dataclass
from io import StringIO
from pathlib import Path
from random import SystemRandom
from shutil import copy
from subprocess import Popen, call, PIPE
from sys import argv, exit
import tomllib

import pylnk3


@dataclass
class DeployFile:
    """A file to deploy to an install location."""

    def __init__(self, src: Path, dest: Path):
        self.src = Path(src)
        self.dest = Path(dest)

    def __repr__(self):
        return f'{type(self).__name__}(src={self.src}, dest={self.dest})'

    def remove_dest(self):
        """Remove the destination file, doing nothing if it doesn't exist."""
        self.dest.unlink(True)

    def deploy(self):
        """Deploy this file to it's install location."""
        self.remove_dest()
        copy(self.src, self.dest)


class LnkDeployFile(DeployFile):
    def deploy(self):
        """Create the lnk file at the install location (`self.dest`), targeting the src location.

        Importantly, the source file for this should be an installed file, not a file that only
        exists in development.
        """
        self.remove_dest()
        # pylnk3 doesn't like `pathlib.Path`s so we `.__fspath__()` them. It might also assume (&
        # require) windows-style backslash-separated paths? I didn't look to hard at the code but it
        # kinda seems like it doesn't normalize them itself?
        pylnk3.for_file(self.src.__fspath__(), self.dest.__fspath__(),
                        work_dir=self.src.parent.__fspath__())


project_dir = Path(__file__).parent.absolute()
# We get the exe name from the Cargo.toml
project_name = tomllib.loads(
    (project_dir/'Cargo.toml').read_text())['package']['name']
exe_name = f'{project_name}.exe'
install_dir = Path('~/AppData/Local', project_name).expanduser()
startup_dir = Path(
    '~/AppData/Roaming/Microsoft/Windows/Start Menu/Programs/Startup').expanduser()

target_dir = Path(os.environ.get('CARGO_TARGET_DIR', './target'))
exe_file = DeployFile(target_dir/'release'/exe_name, install_dir/exe_name)
dot_env_file = DeployFile(project_dir/'.env', install_dir/'.env')
lnk_file = LnkDeployFile(exe_file.dest, startup_dir/f'{project_name}.lnk')
files_to_deploy = [exe_file, dot_env_file, lnk_file]

allowed_args = ['-h', '--help', '-k', '--kill', '--generate-key', '--install-location', '--install-only',
                '--uninstall']


class ProcessKillError(Exception):
    pass


def print_usage():
    """Print the usage information."""
    print('\n'.join(('This script will start the program and add it to the startup directory',
                     '\n\nusage: py deploy_win.py [options]',
                     '\noptions:',
                     '    -h, --help            Show this help information',
                     (f'    -k, --kill            Kill the {project_name} process currently running (if there is one) '
                      'then exit'),
                     '    --generate-key        Generate a key',
                     '    --install-location    Print the installation location (not including the startup folder)',
                     ('    --install-only        Only install the files and set to run at startup, but don\'t start '
                      'the server'),
                     '    --uninstall           Uninstall the program')))


def exit_with_err(msg: str):
    """Print and error then exit."""
    print(f'Error: {msg}')
    exit()


def kill_process(name: str) -> bool:
    """Kill a process.

    Returns:
        bool: `True` if the process was running, `False` if it wasn't.
    """
    with StringIO() as buf, redirect_stdout(buf):
        p = Popen(['taskkill', '/f', '/im', name], stdout=PIPE, stderr=PIPE)
        err_msg = p.stderr.read()
        if err_msg:
            if err_msg == f'ERROR: The process "{name}" not found.\r\n'.encode('utf-8'):
                return False
            raise ProcessKillError(err_msg)

    return True



def generate_key() -> str:
    """Generate a (32 ascii character) key.

    The key is composed of printable ascii characters and will never start or end with a space. The
    requirements of the specifics of the composition of the key come from the Shortcuts client.
    """
    # 32 byte and printable ascii is required because of the Shortcuts client. Not beginning or
    # ending with spaces is a usability consideration.
    rand = SystemRandom()
    while True:
        out = "".join(chr(rand.randrange(32, 127)) for _ in range(32))
        # keys can't begin or end with a space
        if not (out.startswith(" ") or out.endswith(" ")):
            return out



def build():
    """Build the server."""
    call(['cargo', 'build', '--release', '--features', 'no_term'])


def handle_invalid_config():
    """Warn & exit if the server config is invalid.."""
    if os.getenv('REMOTE_CONTROL_KEY') is None and 'REMOTE_CONTROL_KEY' not in dot_env_file.src.read_text():
        exit_with_err(
            'no environment variable set or presence in `.env` for `REMOTE_CONTROL_KEY`')


def handle_invalid_args(args: list[str]):
    """Handle invalid args, including exiting."""
    for arg in args:
        if arg not in allowed_args:
            exit_with_err(
                f"invalid arg '{arg}'\nUse '-h' to list available args.")


def handle_early_exit_args(args: list[str]):
    """Handle args that require early exiting. This includes doing the exiting."""
    if '-h' in args or '--help' in args:
        print_usage()
        exit()
    if '-k' in args or '--kill' in args:
        if kill_process(exe_name):
            print('Killed process')
        else:
            print('Process wasn\'t running')
        exit()
    if '--generate-key' in args:
        line = '-' * 32
        print((f'\n{line}\n\n\x1b[34m{generate_key()}\x1b[0m\n\n{line}\n'
                '^^^^^^  This is your key  ^^^^^^\n'
                '      (between the lines)\n'))
        exit()
    if '--install-location' in args:
        print(install_dir)
        exit()
    if '--uninstall' in args:
        if not install_dir.exists():
            print(f'{project_name} not installed')
            exit()
        if kill_process(exe_name):
            print('Killed process')
        for deploy_file in files_to_deploy:
            deploy_file.remove_dest()
        install_dir.rmdir()
        print('Uninstalled all files')
        exit()


def main():
    args = argv[1:]
    handle_invalid_config()
    handle_invalid_args(args)
    handle_early_exit_args(args)
    # We set the working directory so that cargo works properly when the deploy script is called
    # from somewhere other than the project folder.
    os.chdir(project_dir)
    install_only = '--install-only' in args

    install_dir.mkdir(exist_ok=True)
    build()

    process_killed_msg = ''
    # We don't need to kill the old process if we're just install the files.
    if not install_only:
        if kill_process(exe_name):
            process_killed_msg = ' & old process was killed'

    for deploy_file in files_to_deploy:
        deploy_file.deploy()

    if install_only:
        success_msg = "Files installed"
    else:
        success_msg = f'Process {exe_name} started'
        os.startfile(exe_file.dest)

    print(success_msg + process_killed_msg)


if __name__ == '__main__':
    main()
