#include <stdio.h>

#include "m_argv.h"

#include "doomgeneric.h"

pixel_t* DG_ScreenBuffer = NULL;

void M_FindResponseFile(void);
void D_DoomMain (void);


void doomgeneric_Create(int argc, char **argv)
{
	// save arguments
    myargc = argc;
    myargv = argv;

	M_FindResponseFile();

	DG_ScreenBuffer = malloc(DOOMGENERIC_RESX * DOOMGENERIC_RESY * 4);
	printf("[DBG] DG_ScreenBuffer = %p .. %p (size %u)\n",
	       (void*)DG_ScreenBuffer,
	       (void*)((char*)DG_ScreenBuffer + DOOMGENERIC_RESX * DOOMGENERIC_RESY * 4),
	       (unsigned)(DOOMGENERIC_RESX * DOOMGENERIC_RESY * 4));

	DG_Init();

	D_DoomMain ();
}

