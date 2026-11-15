# Builds corpus_<seed>.elf for each seed in SEEDS: corpus.c plus the
# functions gen.py writes to gen_<seed>.c, compiled for the device the same
# way examples/c-stress is (the SDK's common.mk).
#   make SEEDS="0 1 2 3 18"   (needs PLAYDATE_SDK_PATH and arm-none-eabi-gcc)
# crates/pdsim-arm/tests/lockstep.rs runs the ELFs on both backends.
SEEDS ?= 0 1 2 3 18
COUNT ?= 200

HEAP_SIZE      = 8388208
STACK_SIZE     = 61800

PRODUCT = ThumbCorpus.pdx

SDK = ${PLAYDATE_SDK_PATH}
ifeq ($(SDK),)
$(error SDK path not found; set ENV value PLAYDATE_SDK_PATH)
endif

ifeq ($(SEED),)

.PHONY: corpus clean-corpus
corpus: $(foreach s,$(SEEDS),corpus_$(s).elf)

.SECONDARY:

gen_%.c: gen.py
	python3 gen.py --seed $* --count $(COUNT) --out $@

corpus_%.elf: gen_%.c corpus.c
	$(MAKE) SEED=$* OBJDIR=build/$* build/$*/pdex.elf
	cp build/$*/pdex.elf $@

clean-corpus:
	rm -rf build corpus_*.elf gen_*.c

else

SRC = corpus.c gen_$(SEED).c
UINCDIR =
UASRC =
UDEFS =
UADEFS =
ULIBDIR =
ULIBS =

include $(SDK)/C_API/buildsupport/common.mk

endif
