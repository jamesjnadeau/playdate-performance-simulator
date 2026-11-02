# Builds Source/pdex.elf (device) with the Playdate SDK:
#   make device        (needs PLAYDATE_SDK_PATH and arm-none-eabi-gcc)
HEAP_SIZE      = 8388208
STACK_SIZE     = 61800

PRODUCT = BenchC.pdx

SDK = ${PLAYDATE_SDK_PATH}
ifeq ($(SDK),)
$(error SDK path not found; set ENV value PLAYDATE_SDK_PATH)
endif

VPATH += src
SRC = src/main.c
UINCDIR =
UASRC =
UDEFS =
UADEFS =
ULIBDIR =
ULIBS =

include $(SDK)/C_API/buildsupport/common.mk
