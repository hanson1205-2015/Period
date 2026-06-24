Tools
=====

Command Line
------------

``period [file.period]``
   Compile and run a Period source file. With no file, start the REPL.

``period --lsp``
   Start the Language Server Protocol service.

VS Code Extension
-----------------

The ``period-language`` extension provides:

* Syntax highlighting
* Hover information
* Autocompletion
* Document formatting
* Go to definition
* Diagnostics via LSP

Install it from the release ``.vsix``::

   code --install-extension period-language.vsix
