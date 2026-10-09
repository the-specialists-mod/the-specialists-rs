#include "xash3d_types.h"
#include "const.h"
#include "cvardef.h"
#include "entity_state.h"
#include "q_client.h"     /* usercmd_t */
#include "weaponinfo.h"
#include "pmove.h"         /* pmtrace_t, must precede pm_defs.h */
#include "pm_defs.h"
#include "progdefs.h"
#include "edict.h"
#include "eiface.h"        /* enginefuncs_t, DLL_FUNCTIONS, NEW_DLL_FUNCTIONS, globalvars_t */
#include "cdll_int.h"      /* cl_enginefunc_t, client_data_t, ... */
#include "ref_params.h"
#include "cl_entity.h"
