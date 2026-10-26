@echo off
rem acmodel on Windows: acmodel.cmd <command>
rem Starts the launcher with the py launcher; the launcher re-runs itself under
rem the configured venv's Python and its exit code is passed through.
py -3 -I -B "%~dp0acmodel.py" %*
exit /b %ERRORLEVEL%
