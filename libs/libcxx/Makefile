# libcxx — Minimal C++ Standard Library for anyOS (x86_64)
# Depends on libc64 for malloc/free/printf/memcpy etc.

CXX      = clang++
AR       = ar
CXXFLAGS = --target=x86_64-unknown-none-elf -ffreestanding -nostdlib \
           -fno-exceptions -fno-rtti -std=c++20 -O2 \
           -I include -I ../libc64/include -Wall -Wextra -Wno-unused-parameter $(EXTRA_CXXFLAGS)

SRCDIR = src
OBJDIR = obj

SOURCES = $(wildcard $(SRCDIR)/*.cpp)
OBJECTS = $(patsubst $(SRCDIR)/%.cpp,$(OBJDIR)/%.o,$(SOURCES))

TARGET = libcxx.a

.PHONY: all clean

all: $(TARGET)

$(TARGET): $(OBJECTS)
	$(AR) rcs $@ $^

$(OBJDIR)/%.o: $(SRCDIR)/%.cpp | $(OBJDIR)
	$(CXX) $(CXXFLAGS) -c $< -o $@

$(OBJDIR):
	mkdir -p $(OBJDIR)

clean:
	rm -rf $(OBJDIR) $(TARGET)
