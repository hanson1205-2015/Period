Syntax Guide
============

Statements
----------

Every statement ends with a period (``.``).

.. code-block:: text

   let x be 10.
   show x.

Variables
---------

Declare a variable with ``let ... be``. Reassign with ``set ... to``.

.. code-block:: text

   let name be "Ada".
   set name to "Grace".

Conditionals
------------

.. code-block:: text

   if score >= 60 then.
       show "pass".
   otherwise.
       show "fail".
   end if.

Loops
-----

.. code-block:: text

   let i be 0.
   while i < 10 repeat.
       show i.
       set i to i + 1.
   end while.

Functions
---------

.. code-block:: text

   define factorial with n.
       if n <= 1 then.
           return 1.
       end if.
       return n * factorial with n - 1.
   end define.

Operators
---------

* Arithmetic: ``+``, ``-``, ``*``, ``/``, ``%``, ``**``
* Comparison: ``==``, ``!=``, ``<``, ``>``, ``<=``, ``>=``
* Logical: ``and``, ``or``, ``not``
