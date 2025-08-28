import os
import subprocess

import flexitest
from locust import events
from locust.env import Environment, LocalRunner
from locust.log import setup_logging

from load.cfg import LoadConfig
from typing import Optional


# TODO(load): enhance it to be able to increase/decrease the load dynamically from test runtime.
class LoadGeneratorService(flexitest.Service):
    """
    A separate flexitest service that is able to generate the load as specified by `LoadConfig`.
    """

    env: Environment
    """Locust Environment for running the load against."""

    runner: LocalRunner
    """Locust local runner that actually dispatches the load."""

    cfg: LoadConfig
    """A config that specifies load params: jobs, host, rate, etc."""

    _is_started: bool
    """Whether the LoadGenerator (as a flexitest service) is started."""

    def __init__(self, datadir, cfg: LoadConfig):
        self._is_started = False
        self.cfg = cfg

        self.env = Environment(user_classes=cfg.jobs, events=events, host=cfg.host)
        self.runner = self.env.create_local_runner()

        # Setup service level logging.
        service_logfile = os.path.join(datadir, "service.log")
        log_level = os.getenv("LOG_LEVEL", "DEBUG").upper()
        setup_logging(log_level, logfile=service_logfile)

        # Dirty hack...
        # Injecting datadir directly onto env object.
        # With this, the load jobs are able to set up its logging properly.
        self.env._datadir = datadir

    def start(self):
        self.env.events.init.fire(environment=self.env, runner=self.runner)

        self.runner.start(len(self.cfg.jobs), spawn_rate=self.cfg.spawn_rate)
        self._is_started = True

    def stop(self):
        self._is_started = False
        self.runner.quit()

    def is_started(self):
        return self._is_started

    def check_status(self):
        return True

class DisposableService(flexitest.Service):
    """
    Runs a single command and exits. Not long-lived; no start()/stop() lifecycle.
    Stores only the result of the last run.
    """

    def __init__(self, props: dict, stdout=None):
        super().__init__(props)
        self.stdout = stdout
        self._last_result = None
        self.status_msg = None


    def basic_runner(
        self,
        cmd: list[str],
        env: Optional[dict] = None,
        timeout: Optional[float] = None,
        capture_output: bool = False,  # if True, capture to result.stdout/stderr
        stdout = None,  # override default
        stderr_to_stdout: bool = True,
        input = None
    ) -> subprocess.CompletedProcess:
        """
        Execute a command once and return its CompletedProcess.
        - If capture_output=True, stdout captured in memory
        - otherwise, output goes to provided stdout, And if that is also
        not provided then the parent process' stdio.
        - If stderr_to_stdout=True and we're not capturing,
        stderr is redirected to stdout
        """
        self._reset_state()

        # Build env without clobbering the parent environment
        if env is not None:
            merged_env = os.environ.copy()
            merged_env.update(env)
        else:
            merged_env = None

        run_kwargs = dict(timeout=timeout, input=input, env=merged_env)

        # Output routing
        if capture_output:
            run_kwargs["stdout"] = subprocess.PIPE
            run_kwargs["stderr"] = subprocess.STDOUT if stderr_to_stdout else subprocess.PIPE
            close_file = None
        else:
            sink = stdout if stdout is not None else self.stdout
            close_file = None
            if isinstance(sink, str):
                # Append like your ProcService; caller owns the path, we own the handle here.
                f = open(sink, "a", buffering=1)  # line-buffered if possible
                f.write(f"(process started as: {cmd})\n")
                run_kwargs["stdout"] = f
                run_kwargs["stderr"] = f if stderr_to_stdout else None
                close_file = f
            elif sink is not None:
                run_kwargs["stdout"] = sink
                run_kwargs["stderr"] = sink if stderr_to_stdout else None
            else:
                # inherit parent's stdio (no kwargs)
                pass

        try:
            result = subprocess.run(cmd, **run_kwargs)
            self._last_result = result
            self._update_status_msg(result)
            return result
        finally:
            if close_file is not None:
                try:
                    close_file.close()
                except Exception:
                    pass

    def get_status_msg(self):
        return self.status_msg

    def last_returncode(self):
        return None if self._last_result is None else self._last_result.returncode

    def last_output(self):
        """Convenience: returns captured stdout if capture_output=True was used."""
        if self._last_result is None:
            return None
        return self._last_result.stdout

    # start and stop methods aren't to be used as this is not
    def start(self):
        pass

    def stop(self):
        pass

    def is_started(self) -> bool:
        # temp state so we can always return false
        return False

    def check_status(self) -> bool:
        # No background process to poll; always False.
        return False

    def _update_status_msg(self, result: subprocess.CompletedProcess):
        self.status_msg = f"code:{result.returncode}"
