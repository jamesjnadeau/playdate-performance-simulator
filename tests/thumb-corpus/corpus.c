// Runs the generated corpus (gen_<seed>.c, see gen.py) once per frame.
#include "pd_api.h"

void corpus_run(uint32_t frame);

static uint32_t frame;

static int update(void *userdata)
{
    (void)userdata;
    corpus_run(frame++);
    return 1;
}

#ifdef _WINDLL
__declspec(dllexport)
#endif
int eventHandler(PlaydateAPI *pd, PDSystemEvent event, uint32_t arg)
{
    (void)arg;
    if (event == kEventInit)
        pd->system->setUpdateCallback(update, pd);
    return 0;
}
