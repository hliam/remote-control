# To use this, you must have a `.env` file that contains a `KEY` environment variable.

# TODO:
#  - finish install flag stuff
#  - finish uninstall flag stuff

import os
from contextlib import suppress, redirect_stdout
from dataclasses import dataclass
from io import StringIO
from pathlib import Path
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
        """Try to remove the destination file, doing nothing if it doesn't exist."""
        with suppress(FileNotFoundError):
            self.dest.unlink()

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

exe_file = DeployFile(project_dir/'target/release' /
                      exe_name, install_dir/exe_name)
dot_env_file = DeployFile(project_dir/'.env', install_dir/'.env')
lnk_file = LnkDeployFile(exe_file.dest, startup_dir/f'{project_name}.lnk')
files_to_deploy = [exe_file, dot_env_file, lnk_file]


class ProcessKillError(Exception):
    pass


class ProcessNotRunningError(ProcessKillError):
    pass


def print_usage():
    print('\n'.join(('This script will start the program and add it to the startup directory',
                     '\n\nusage: py deploy_win.py [options]',
                     '\noptions:',
                     '    -h, --help    Show this help information.',
                    f'    -k, --kill    Kill the {project_name} process currently running (if there is one) then exit.'
                     '    --install-only    Only install the files, without starting the server'
                     '    --uninstall    Uninstall the program.')))


def exit_with_err(msg: str):
    print(f'Error: {msg}')
    exit()


def kill_process(name: str):
    with StringIO() as buf, redirect_stdout(buf):
        p = Popen(['taskkill', '/f', '/im', name], stdout=PIPE, stderr=PIPE)
        err_msg = p.stderr.read()
        if err_msg:
            if err_msg == f'ERROR: The process "{name}" not found.\r\n'.encode('utf-8'):
                raise ProcessNotRunningError()
            raise ProcessKillError(err_msg)


def build():
    call(['cargo', 'build', '--release', '--features', 'no_term'])


def handle_invalid_config():
    if os.getenv('REMOTE_CONTROL_KEY') is None and 'REMOTE_CONTROL_KEY' not in dot_env_file.src.read_text():
        exit_with_err(
            'no environment variable set or presence in `.env` for `REMOTE_CONTROL_KEY`')


def handle_early_exit_args():
    args = argv[1:]
    if '-h' in args or '--help' in args:
        print_usage()
        exit()
    if '-k' in args or '--kill' in args:
        try:
            kill_process(exe_name)
            print('Killed process')
        except ProcessNotRunningError:
            print('Process wasn\'t running')
        exit()


def main():
    handle_invalid_config()
    handle_early_exit_args()
    # We set the working directory so that cargo works properly when the deploy script is called
    # from somewhere other than the project folder.
    os.chdir(project_dir)

    with suppress(FileExistsError):
        install_dir.mkdir()
    build()

    success_msg = f'Process {exe_name} started'
    with suppress(ProcessKillError):
        kill_process(exe_name)
        success_msg += ' & old process was killed'

    for deploy_file in files_to_deploy:
        deploy_file.deploy()

    os.startfile(exe_file.dest)

    print(success_msg)


if __name__ == '__main__':
    main()
