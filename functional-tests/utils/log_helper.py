import contextvars
import functools
import re

log_metadata_var = contextvars.ContextVar("tx_caller_log_metadata", default=None)


def _format_string(template: str, args: tuple):
    """
    Formats a string by replacing [index] placeholders with corresponding args values.
    Indexes should be 1-numbered, because the 0-th arg is self.
    """

    def replacer(match):
        # Extract the number inside brackets
        index = int(match.group(1))
        # Replace if valid index
        return str(args[index]) if index < len(args) else match.group(0)

    return re.sub(r"\[(\d+)\]", replacer, template)


def tx_caller(fmt_str_pattern: str):
    """Decorator to set a context local variable with logging metadata."""

    def decorator(func):
        @functools.wraps(func)
        def wrapper(*args, **kwargs):
            # Setting data only for the parent caller in the chain of calls.
            if log_metadata_var.get() is None:
                # Formatting the logging metadata.
                token_metadata = log_metadata_var.set(_format_string(fmt_str_pattern, args))
            else:
                token_metadata = None

            try:
                return func(*args, **kwargs)
            finally:
                # Reset context variables after function execution to avoid leaking data
                if token_metadata is not None:
                    log_metadata_var.reset(token_metadata)

        return wrapper

    return decorator
