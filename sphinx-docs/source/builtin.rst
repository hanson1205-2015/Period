Built-in Functions
==================

Period provides a small set of built-in functions.

``length with value``
   Returns the length of a string, list, or dictionary.

``string with value``
   Converts a value to a string.

``number with value``
   Converts a value to a number.

``type with value``
   Returns the type name of a value.

``input``
   Reads one line from standard input.

Examples
--------

.. code-block:: text

   let name be input.
   show "Hello, " + name + "!".

   let items be [1, 2, 3].
   show length with items.
