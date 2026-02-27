# libc++abi — Itanium C++ ABI runtime for anyOS (x86_64)
# Depends on libc64 + libunwind

CXX      = clang++
AR       = ar
CXXFLAGS = --target=x86_64-unknown-none-elf -ffreestanding -nostdlib \
           -fno-exceptions -std=c++20 -O2 \
           -I include -I ../libc64/include -I ../libunwind/include -I ../libcxx/include \
           -Wall -Wextra -Wno-unused-parameter $(EXTRA_CXXFLAGS)

SRCDIR = src
OBJDIR = obj

SOURCES = $(wildcard $(SRCDIR)/*.cpp)
OBJECTS = $(patsubst $(SRCDIR)/%.cpp,$(OBJDIR)/%.o,$(SOURCES))

TARGET = libc++abi.a

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
