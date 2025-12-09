from typing import cast

import flexitest

from common.service import ServiceWrapper


class StrataRunContext(flexitest.RunContext):
    """A wrapper to flexitest's runcontext that overrides get_service to be more typesafe."""
    def get_service(self, name: str):
        svc = super().get_service(name)
        if svc is not None:
            return cast(ServiceWrapper, svc)
        return None
