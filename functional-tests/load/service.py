import os

import flexitest
from locust import events
from locust.env import Environment, LocalRunner
from locust.log import setup_logging

from load.cfg import LoadConfig


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
