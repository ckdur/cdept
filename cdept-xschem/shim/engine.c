/* engine.c: start-up and Tcl access for the helper. It takes the place of xschem's
 * main.c, which is not compiled: no Tk main loop, no interactive shell, no signal
 * handlers, no detaching from the terminal.
 *
 * Part of cdept-xschem, GPL-2.0-or-later.
 */
#include "xschem.h"

/* Start xschem without drawing (has_x = 0). `argv` uses xschem's own command line
 * options; argv[0] is the program name. Returns a Tcl status code (TCL_OK = 0). */
int cdx_init(int argc, char **argv)
{
  int i;

  errfp = stderr;
  setvbuf(stdout, NULL, _IOLBF, 0);
  /* The Rust runtime ignores SIGPIPE and child processes inherit that. xschem runs awk
   * pipelines, which expect the default. Our own socket writes do not raise it. */
  signal(SIGPIPE, SIG_DFL);

  has_x = 0;
  cli_argc = argc;
  cli_opt_argc = process_options(argc, argv);
  has_x = 0; /* whatever the options said: there is no display in this step */

  my_strdup(_ALLOC_ID_, &xschem_executable, argv[0]);
  cli_opt_argv = my_malloc(_ALLOC_ID_, cli_opt_argc * sizeof(char *));
  for(i = 0; i < cli_opt_argc; ++i) {
    cli_opt_argv[i] = NULL;
    my_strdup(_ALLOC_ID_, &cli_opt_argv[i], argv[i]);
  }

  Tcl_FindExecutable(argv[0]);
  interp = Tcl_CreateInterp();
  return Tcl_AppInit(interp);
}

/* Evaluate a script at global level. Returns the Tcl status code; the result or the
 * error message is read with cdx_result(). */
int cdx_eval(const char *script)
{
  return Tcl_EvalEx(interp, script, -1, TCL_EVAL_GLOBAL);
}

/* Result of the last cdx_eval(), valid until the next call into Tcl. */
const char *cdx_result(void)
{
  return Tcl_GetStringResult(interp);
}

/* Run the Tcl events that are ready (timers, file events), without waiting. */
void cdx_run_pending_events(void)
{
  while(Tcl_DoOneEvent(TCL_ALL_EVENTS | TCL_DONT_WAIT)) {
    /* keep going until the queue is empty */
  }
}

const char *cdx_xschem_version(void)
{
  return XSCHEM_VERSION;
}
