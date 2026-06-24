Period Documentation
====================

**Period** is an elegant English programming language where every statement ends with a period.

.. toctree::
   :maxdepth: 2
   :caption: Contents:

   syntax
   builtin
   tools
   api

Overview
--------

Period programs read like short articles. Variables are introduced with ``let``,
functions with ``define``, and every statement is terminated by a period.

.. code-block:: text

   -- Greet the world.
   let greeting be "Hello, World!".
   show greeting.

Features
--------

* Sentence-like syntax.
* Precise, line-and-column error messages.
* Parser recovery that reports multiple errors at once.
* Turing-complete: variables, loops, conditionals, and recursive functions.
* VS Code extension with LSP diagnostics, hover, completion, formatting, and go-to-definition.
* Command-line compiler and interactive REPL.

Quick Start
-----------

Install Period, then run a program::

   period hello.period

Start the interactive REPL::

   period
