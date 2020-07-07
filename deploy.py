import argparse
import os
from contextlib import suppress, redirect_stdout
from io import StringIO
from pathlib import Path
from shutil import copy
from subprocess import check_call, DEVNULL, Popen, PIPE
from sys import exit, platform

try:
    import crontab
except ImportError:
    if not platform == 'win32':
        raise


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

    def __init__(self, src: Path, dst: Path = None, full_src_path: bool = False, full_dst_path: bool = False):
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
    def __init__(self, message):
        self.message = message


# for macOS, python-crontab is used. for windows, we put a symlink in the startup folder.
# TODO: change this comment to reflect whatever ends up in the startup folder (not a symlink)
class Scheduler:
    if not platform == 'win32':
        _cron = crontab.CronTab(True)

    @classmethod
    def _raise_unscheduled_error(cls):
        raise ValueError('no job is scheduled')

    @classmethod
    def _get_command(cls) -> str:
        return str(start_script.dst)

    @classmethod
    def _find_cron_command(cls):
        try:
            return next(cls._cron.find_command(cls._get_command()))
        except StopIteration:
            return None

    @classmethod
    def is_scheduled(cls) -> bool:
        if platform == 'win32':
            return start_script.dst.exists()
        return cls._find_cron_command() is not None

    @classmethod
    def schedule(cls):
        if platform == 'win32':
            start_script.deploy()
        else:
            with cls._cron as cron:
                cron.new(command=cls._get_command()).every_reboot()

    @classmethod
    def unschedule(cls):
        if platform == 'win32':
            try:
                start_script.dst.unlink()
            except FileNotFoundError:
                cls._raise_unscheduled_error()
        else:
            try:
                with cls._cron as cron:
                    cron.remove(cls._find_cron_command())
            except TypeError:
                cls._raise_unscheduled_error()


def exit_with_err(msg: str):
    print(f'[error] {msg}')
    exit()


def print_info(msg: str):
    print(f'[info] {msg}')


def kill_process(name: str):
    with StringIO() as buf, redirect_stdout(buf):
        if platform == 'win32':
            args = ['taskkill', '/f', '/im', name]
        else:
            if not Popen(['pgrep', name], stdout=PIPE).stdout.read():
                raise ProcessKillError(f'process isn\'t running')
            args = ['pkill', '-9', name]

        err_msg = Popen(args, stdout=PIPE, stderr=PIPE).stderr.read()
        if err_msg:
            raise ProcessKillError(f'something went wrong trying to kill the process ({buf!s})')


def build():
    check_call(['cargo', 'build', '--release', '--manifest-path', str(DeployFile.src_base / 'Cargo.toml')])


def get_args():
    parser = argparse.ArgumentParser()
    parser.add_argument('-k', '--kill', action='store_true',
                        help='kill the running remote-control process then exit')
    parser.add_argument('--uninstall', action='store_true',
                        help='kill the running process, then remove all related files outside of this repository')
    parser.add_argument('-l', '--location', action='store_true',
                        help='get the installation location')
    return parser.parse_args()


def handle_invalid_config():
    try:
        if os.getenv('KEY') is None and 'KEY' not in dot_env_file.src.read_text():
            exit_with_err('no environment variable set or present in `.env` for `KEY`')
    except FileNotFoundError:
        exit_with_err("no environment variable set and no `.env` file in project folder")
    if not rocket_config_file.src.exists():
        exit_with_err('no `Rocket.toml` present')


def execute_file(path: os.PathLike, in_background=False):
    if in_background:
        Popen([path], shell=False, stdout=DEVNULL)
    else:
        check_call([path], shell=False)


def uninstall():
    with suppress(ProcessKillError):
        kill_process(exe_name)
        print_info('process killed')

    for file in files_to_deploy:
        with suppress(FileNotFoundError):
            file.dst.unlink()

    with suppress(FileNotFoundError):
        file.dst_base.rmdir()
        print_info('removed deployment files and directory')

    with suppress(ValueError):
        Scheduler.unschedule()
        print_info('unscheduled process run at startup')

    print_info(f'{name} has been uninstalled')


def deploy():
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
    print_info(f'process started')


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
        deploy()


if __name__ == '__main__':
    main()
