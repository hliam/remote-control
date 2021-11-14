from abc import ABC
import argparse
import os
from contextlib import suppress, redirect_stdout
from io import StringIO
from pathlib import Path
from shutil import copy
from subprocess import check_call, DEVNULL, Popen, PIPE
from sys import exit, platform

if not platform == 'win32':
    import crontab


name = 'remote_control'


class DeployFile:
    """A file that has a source (copy from) and a destination (copy to).

    `src` and `dst` should be relative paths that are children of the `DeployFile.src_base` and `DeployFile.dst_base`
    directories.

    Args:
        src (os.PathLike): A relative path that is a child of `DisplayFile.src_base`.
        dst (:obj:`os.PathLike`, optional): A relative path that is a child of `DisplayFile.dst_base`. If not present,
            the same path as `src` will be used.
    """

    src_base = Path(__file__).parent.absolute()
    if platform == 'win32':
        dst_base = Path('~/AppData/Local', name).expanduser()
    else:
        dst_base = Path('~/Library/Application Support', name).expanduser()

    def __init__(
        self, src: os.PathLike,
        dst: os.PathLike = None,
        full_src_path: bool = False,
        full_dst_path: bool = False
    ):
        if dst is None:
            dst = src
        self.src = src if full_src_path else DeployFile.src_base / src
        self.dst = dst if full_dst_path else DeployFile.dst_base / dst

    def __repr__(self):
        return f'DeployFile(src={self.src!r}, dst={self.dst!r})'

    def deploy(self):
        self.remove()
        with suppress(FileNotFoundError):
            copy(self.src, self.dst)

    def remove(self):
        with suppress(FileNotFoundError):
            self.dst.unlink()


if platform == 'win32':
    exe_name = f'{name}.exe'
else:
    exe_name = name

exe_file = DeployFile(Path('target/release', exe_name), exe_name)
dot_env_file = DeployFile('.env')
rocket_config_file = DeployFile('Rocket.toml')
files_to_deploy = [exe_file, rocket_config_file, dot_env_file]

if platform == 'win32':
    startup_folder = Path('~/AppData/Roaming/Microsoft/Windows/Start Menu/Programs/Startup').expanduser()
    start_script = DeployFile('start.bat', startup_folder / 'start_remote_control.bat')
else:
    files_to_deploy.append(DeployFile('macos_minimize_windows.applescript'))
    start_script = DeployFile('start.sh')
    files_to_deploy.append(start_script)


class ProcessKillError(Exception):
    def __init__(self, message: str):
        self.message = message


class NotScheduledError(Exception):
    def __init__(self):
        super().__init__('No job is scheduled.')


# On windows, we put a symlink in the startup folder to schedule running at startup.
# TODO: change this comment to reflect whatever ends up in the startup folder (not a symlink)
class _WindowsScheduler:
    @classmethod
    def is_scheduled(cls) -> bool:
        return start_script.dst.exists()

    @classmethod
    def schedule(cls):
        start_script.deploy()

    @classmethod
    def unschedule(cls):
        try:
            start_script.dst.unlink()
        except FileNotFoundError:
            raise NotScheduledError()


# On macOS, we use python-crontab to schedule running at startup.
class _MacOSScheduler:
    if not platform == 'win32':
        self._cron = crontab.CronTab(True)

    @classmethod
    def _get_command(cls) -> str:
        return str(start_script.dst)

    @classmethod
    def _get_cron_command(cls):
        try:
            return next(cls._cron.find_command(cls._get_command()))
        except StopIteration:
            return None

    @classmethod
    def is_scheduled(cls) -> bool:
        return cls._get_cron_command() is not None

    @classmethod
    def schedule(cls):
        with cls._cron as cron:
            cron.new(command=cls._get_command()).every_reboot()

    @classmethod
    def unschedule(cls):
        try:
            cron.remove(cls._get_cron_command())
        except TypeError:
            raise NotScheduledError()


Scheduler = _WindowsScheduler if platform == 'win32' else _MacOSScheduler


def exit_with_err(msg: str):
    """Print an error to the console then exit."""
    print(f'[error] {msg}')
    exit()


def print_info(msg: str):
    """Print general information to console."""
    print(f'[info] {msg}')


def kill_process(name: str):
    """Kill the remote-control process.

    Raises:
        ProcessKillError: If the process isn't running or if something
            else went wrong trying to kill it.
    """
    with StringIO() as buf, redirect_stdout(buf):
        if platform == 'win32':
            args = ['taskkill', '/f', '/im', name]
        else:
            # Detect if the process is running.
            if not Popen(['pgrep', name], stdout=PIPE).stdout.read():
                raise ProcessKillError('Process isn\'t running.')
            args = ['pkill', '-9', name]

        err_msg = Popen(args, stdout=PIPE, stderr=PIPE).stderr.read()
        # There will be an error on windows if the process isn't running, right now it isn't distinguished if the call
        # failed because the process doesn't exist or because something went wrong. This should probably be fixed. TODO.
        if err_msg:
            raise ProcessKillError(f'Something went wrong trying to kill the process ({buf!s}).')


def build():
    """Build with cargo."""
    check_call(['cargo', 'build', '--release', '--manifest-path', str(DeployFile.src_base / 'Cargo.toml')])


def get_args():
    """Get the program arguments."""
    parser = argparse.ArgumentParser()
    parser.add_argument('-k', '--kill', action='store_true',
                        help='kill the running remote-control process then exit')
    parser.add_argument('--uninstall', action='store_true',
                        help='kill the running process, then remove all related files outside of this repository')
    parser.add_argument('-l', '--location', action='store_true',
                        help='get the installation location')
    return parser.parse_args()


def handle_invalid_config():
    """Detect and handle an invalid config.

    This will handle the lack of a key or `rocket.toml` and exit the
    program accordingly.
    """
    if not rocket_config_file.src.exists():
        exit_with_err('No `Rocket.toml` present.')

    with suppress(FileNotFoundError):
        if os.getenv('KEY') is not None or 'KEY' in dot_env_file.src.read_text():
            return

    exit_with_err("Key not in environment or `.env` file.")


def execute_file(path: os.PathLike, in_background: bool = False):
    """Execute a file.

    This function blocks unless `in_background` is true.
    """
    if in_background:
        Popen([path], shell=False, stdout=DEVNULL)
    else:
        check_call([path], shell=False)


def uninstall():
    """Uninstall remote control, deleting all files.

    This will kill the process, remove installed files, and unschedule
    run at startup.
    """
    with suppress(ProcessKillError):
        kill_process(exe_name)
        print_info('process killed')

    for file in files_to_deploy:
        with suppress(FileNotFoundError):
            file.dst.unlink()

    with suppress(FileNotFoundError):
        file.dst_base.rmdir()
        print_info('removed deployment files and directory')

    with suppress(NotScheduledError):
        Scheduler.unschedule()
        print_info('unscheduled run at startup')

    print_info(f'{name} has been uninstalled')


def deploy_all():
    """Deploy remote control.

    This will kill previous instances, build the program, deploy
    necessary filed to the install location, then run the program.
    """
    with suppress(FileExistsError):
        DeployFile.dst_base.mkdir()

    build()

    with suppress(ProcessKillError):
        kill_process(exe_name)
        print_info('old process killed')

    for deploy_file in files_to_deploy:
        deploy_file.deploy()

    if not Scheduler.is_scheduled():
        Scheduler.schedule()
        print_info('scheduled process to run at startup')

    execute_file(exe_file.dst, True)
    print_info('process started')


def main():
    args = get_args()
    handle_invalid_config()

    if args.location:
        print(DeployFile.dst_base)
        exit()
    elif args.kill:
        try:
            kill_process(exe_name)
            print_info('process killed')
        except ProcessKillError:
            print_info('process not running')
    elif args.uninstall:
        uninstall()
    else:
        deploy_all()


if __name__ == '__main__':
    main()
