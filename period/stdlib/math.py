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
    """Return the sine of x (x in radians)."""
    return _math.sin(x)


def cos(x):
    """Return the cosine of x (x in radians)."""
    return _math.cos(x)


def tan(x):
    """Return the tangent of x (x in radians)."""
    return _math.tan(x)


def sqrt(x):
    """Return the square root of x."""
    return _math.sqrt(x)


def floor(x):
    """Return the largest integer less than or equal to x."""
    return _math.floor(x)


def ceil(x):
    """Return the smallest integer greater than or equal to x."""
    return _math.ceil(x)


def abs(x):
    """Return the absolute value of x."""
    return _math.fabs(x)


def round(x):
    """Return x rounded to the nearest integer."""
    return _math.floor(x + 0.5)


# Hover documentation: tuple of (signature, docstring) or a plain docstring.
DOCS = {
    "pi": "The ratio of a circle's circumference to its diameter (3.14159...).",
    "e": "The base of the natural logarithm (2.71828...).",
    "sin": ("sin with <x> -> number", "Return the sine of x (x in radians)."),
    "cos": ("cos with <x> -> number", "Return the cosine of x (x in radians)."),
    "tan": ("tan with <x> -> number", "Return the tangent of x (x in radians)."),
    "sqrt": ("sqrt with <x> -> number", "Return the square root of x."),
    "floor": ("floor with <x> -> number", "Return the largest integer less than or equal to x."),
    "ceil": ("ceil with <x> -> number", "Return the smallest integer greater than or equal to x."),
    "abs": ("abs with <x> -> number", "Return the absolute value of x."),
    "round": ("round with <x> -> number", "Return x rounded to the nearest integer."),
}
