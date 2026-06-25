"""Math utilities for Period."""
import math as _math

EXPORTS = [
    "pi",
    "e",
    "sin",
    "cos",
    "tan",
    "sqrt",
    "floor",
    "ceil",
    "abs",
    "round",
]

pi = _math.pi
e = _math.e


def sin(x):
    return _math.sin(x)


def cos(x):
    return _math.cos(x)


def tan(x):
    return _math.tan(x)


def sqrt(x):
    return _math.sqrt(x)


def floor(x):
    return _math.floor(x)


def ceil(x):
    return _math.ceil(x)


def abs(x):
    return _math.fabs(x)


def round(x):
    return _math.floor(x + 0.5)
