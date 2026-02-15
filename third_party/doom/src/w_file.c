//
// Copyright(C) 1993-1996 Id Software, Inc.
// Copyright(C) 2005-2014 Simon Howard
//
// This program is free software; you can redistribute it and/or
// modify it under the terms of the GNU General Public License
// as published by the Free Software Foundation; either version 2
// of the License, or (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// DESCRIPTION:
//	WAD I/O functions.
//

#include <stdio.h>

#include "config.h"

#include "doomtype.h"
#include "doomgeneric.h"
#include "m_argv.h"

#include "w_file.h"

extern wad_file_class_t stdc_wad_file;

/*
#ifdef _WIN32
extern wad_file_class_t win32_wad_file;
#endif
*/

#ifdef HAVE_MMAP
extern wad_file_class_t posix_wad_file;
#endif 

static wad_file_class_t *wad_file_classes[] = 
{
/*
#ifdef _WIN32
    &win32_wad_file,
#endif
*/
#ifdef HAVE_MMAP
    &posix_wad_file,
#endif
    &stdc_wad_file,
};

wad_file_t *W_OpenFile(char *path)
{
    wad_file_t *result;
    int i;

    //!
    // Use the OS's virtual memory subsystem to map WAD files
    // directly into memory.
    //

    if (!M_CheckParm("-mmap"))
    {
        return stdc_wad_file.OpenFile(path);
    }

    // Try all classes in order until we find one that works

    result = NULL;

    for (i = 0; i < arrlen(wad_file_classes); ++i)
    {
        result = wad_file_classes[i]->OpenFile(path);

        if (result != NULL)
        {
            break;
        }
    }

    return result;
}

void W_CloseFile(wad_file_t *wad)
{
    wad->file_class->CloseFile(wad);
}

size_t W_Read(wad_file_t *wad, unsigned int offset,
              void *buffer, size_t buffer_len)
{
    if (wad == NULL) {
        printf("[DBG] W_Read: FATAL wad is NULL! offset=%u len=%u\n",
               offset, (unsigned)buffer_len);
        *((volatile int*)0) = 0; /* deliberate crash with clear message */
    }
    if (wad->file_class == NULL) {
        printf("[DBG] W_Read: FATAL wad=%p has file_class=NULL!\n", (void*)wad);
        printf("[DBG]   wad bytes: %08x %08x %08x %08x\n",
               ((unsigned int*)wad)[0], ((unsigned int*)wad)[1],
               ((unsigned int*)wad)[2], ((unsigned int*)wad)[3]);
        /* Check if this looks like it's inside DG_ScreenBuffer */
        extern pixel_t *DG_ScreenBuffer;
        if (DG_ScreenBuffer) {
            char *sb = (char*)DG_ScreenBuffer;
            char *wp = (char*)wad;
            if (wp >= sb && wp < sb + 320*200*4) {
                printf("[DBG]   wad ptr is INSIDE DG_ScreenBuffer! offset=%d\n",
                       (int)(wp - sb));
            }
        }
        *((volatile int*)0) = 0; /* deliberate crash */
    }
    return wad->file_class->Read(wad, offset, buffer, buffer_len);
}

