from typing import Callable

import flexitest


class EnvControlBuilder:
    """
    Provides standardized interface for service lookup, validation, and retrieval
    so that we can know if something is injectable to the Env or not and then build it
    if it is injectable
    """

    def __init__(self, ctx: flexitest.LiveEnv):
        self.ctx = ctx
        self.service_requirements: dict[str, Callable] = {}

    def get_service(self, service_name: str):
        return self.ctx.get_service(service_name)

    def requires_service(self, service_name: str, transform_lambda: Callable):
        """
        what service you need and function to describe transformation.
        Args:
            service_name: Name of the service to require
            transform_lambda: Lambda function to get exact data
        """

        self.service_requirements[service_name] = transform_lambda
        return self

    def build(self):
        """
        Resolve all service requirements and return transformed configs.

        Returns:
            Dictionary mapping service names to their transformed values
        """
        # Validate all required services exist
        # Transform services using their lambdas
        resolved_configs = {}
        for service_name, transform_lambda in self.service_requirements.items():
            try:
                service = self.get_service(service_name)
                resolved_configs[service_name] = transform_lambda(service)
            except KeyError as err:
                raise ServiceNotAvailable(service_name) from err

        return resolved_configs


class ServiceNotAvailable(Exception):
    def __init__(self, message="env doesn't have that service injected"):
        self.message = message
        super().__init__(self.message)

    def __str__(self):
        return f"ServiceNotAvailable: {self.message}"
