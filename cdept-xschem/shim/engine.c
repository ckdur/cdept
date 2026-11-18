/* engine.c: start-up and Tcl access for the helper. It takes the place of xschem's
 * main.c, which is not compiled: no Tk main loop, no interactive shell, no signal
 * handlers, no detaching from the terminal.
 *
 * Part of cdept-xschem, GPL-2.0-or-later.
 */
#include "xschem.h"
#include "cdx.h"

/* xschem's srand() calls arrive here (see xschem_config.h): the seed it asks for,
 * made different for every process and moment. */
#undef srand
extern void srand(unsigned int seed);
void xs_srand(unsigned int seed)
{
  Tcl_Time now;
  Tcl_GetTime(&now);
  srand(seed ^ ((unsigned int)getpid() * 2654435761u) ^ (unsigned int)now.usec);
}

static void (*ui_sink)(const char *json) = NULL;

/* Tcl: cdept::ui json */
static int ui_command(ClientData data, Tcl_Interp *ip, int objc, Tcl_Obj *const objv[])
{
  (void)data;
  if(objc != 2) {
    Tcl_WrongNumArgs(ip, 1, objv, "json");
    return TCL_ERROR;
  }
  if(ui_sink) ui_sink(Tcl_GetString(objv[1]));
  return TCL_OK;
}

/* See cdx.h. */
int cdx_init(int argc, char **argv, int display)
{
  int i, status;

  errfp = stderr;
  setvbuf(stdout, NULL, _IOLBF, 0);
  /* The Rust runtime ignores SIGPIPE and child processes inherit that. xschem runs awk
   * pipelines, which expect the default. Our own socket writes do not raise it. */
  signal(SIGPIPE, SIG_DFL);

  has_x = 0;
  cli_argc = argc;
  cli_opt_argc = process_options(argc, argv);
  /* Whatever the options said: drawing is the caller's choice. With has_x set, xschem
   * calls the stand-in Xlib and Tk of xshim.c; xschem's own test for an X server is in
   * main.c, which is not compiled. */
  has_x = display ? 1 : 0;

  my_strdup(_ALLOC_ID_, &xschem_executable, argv[0]);
  cli_opt_argv = my_malloc(_ALLOC_ID_, cli_opt_argc * sizeof(char *));
  for(i = 0; i < cli_opt_argc; ++i) {
    cli_opt_argv[i] = NULL;
    my_strdup(_ALLOC_ID_, &cli_opt_argv[i], argv[i]);
  }

  Tcl_FindExecutable(argv[0]);
  interp = Tcl_CreateInterp();
  /* Before Tcl_AppInit, so that everything xschem's start-up runs can use it. */
  Tcl_Eval(interp, "namespace eval cdept {}");
  Tcl_CreateObjCommand(interp, "cdept::ui", ui_command, NULL, NULL);
  /* While xschem starts, nobody is connected who could answer a dialog. The Tcl side
   * (faketk.tcl) does not wait for an answer as long as this is set. */
  Tcl_SetVar(interp, "::cdept::starting", "1", TCL_GLOBAL_ONLY);
  status = Tcl_AppInit(interp);
  Tcl_SetVar(interp, "::cdept::starting", "0", TCL_GLOBAL_ONLY);
  return status;
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

void cdx_set_ui_sink(void (*sink)(const char *json))
{
  ui_sink = sink;
}

static void fd_readable(ClientData data, int mask)
{
  void (*readable)(void) = (void (*)(void))data;
  (void)mask;
  readable();
}

void cdx_watch_fd(int fd, void (*readable)(void))
{
  Tcl_CreateFileHandler(fd, TCL_READABLE, fd_readable, (ClientData)readable);
}

static void idle_call(ClientData data)
{
  void (*call)(void) = (void (*)(void))data;
  call();
}

void cdx_when_idle(void (*call)(void))
{
  Tcl_DoWhenIdle(idle_call, (ClientData)call);
}

int cdx_do_one_event(int wait)
{
  return Tcl_DoOneEvent(TCL_ALL_EVENTS | (wait ? 0 : TCL_DONT_WAIT));
}

const char *cdx_xschem_version(void)
{
  return XSCHEM_VERSION;
}
