/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 *
 * cxa_rtti.cpp — RTTI (Run-Time Type Information) support for anyOS.
 *
 * Provides the definitions for std::type_info and the __cxxabiv1 type
 * info classes required by the Itanium C++ ABI.
 */

#include <typeinfo>
#include <cxxabi.h>
#include <string.h>

/* ── std::type_info virtual destructor ────────────────────────────────── */

namespace std {
type_info::~type_info() {}
} // namespace std

/* ── __cxxabiv1 RTTI class implementations ────────────────────────────── */

namespace __cxxabiv1 {

__class_type_info::__class_type_info(const char *name)
    : std::type_info(name) {}
__class_type_info::~__class_type_info() {}

__si_class_type_info::__si_class_type_info(const char *name,
                                            const __class_type_info *base)
    : __class_type_info(name), __base_type(base) {}
__si_class_type_info::~__si_class_type_info() {}

__vmi_class_type_info::__vmi_class_type_info(const char *name,
                                              unsigned int flags,
                                              unsigned int base_count)
    : __class_type_info(name), __flags(flags), __base_count(base_count) {}
__vmi_class_type_info::~__vmi_class_type_info() {}

__fundamental_type_info::__fundamental_type_info(const char *name)
    : std::type_info(name) {}
__fundamental_type_info::~__fundamental_type_info() {}

__pointer_type_info::__pointer_type_info(const char *name,
                                          unsigned int flags,
                                          const std::type_info *pointee)
    : std::type_info(name), __flags(flags), __pointee(pointee) {}
__pointer_type_info::~__pointer_type_info() {}

} // namespace __cxxabiv1

/* ── __dynamic_cast — simplified for single/no inheritance ────────────── */

extern "C" void *__dynamic_cast(
    const void *src_ptr,
    const __cxxabiv1::__class_type_info *src_type,
    const __cxxabiv1::__class_type_info *dst_type,
    long src2dst_offset) {
    (void)src2dst_offset;

    if (!src_ptr) return nullptr;

    /* If source and destination types are the same, trivial cast. */
    if (src_type == dst_type || src_type->name() == dst_type->name())
        return const_cast<void *>(src_ptr);

    /* Check single inheritance chain. */
    const auto *si = dynamic_cast<const __cxxabiv1::__si_class_type_info *>(src_type);
    while (si) {
        if (si->__base_type == dst_type ||
            si->__base_type->name() == dst_type->name()) {
            return const_cast<void *>(src_ptr);
        }
        si = dynamic_cast<const __cxxabiv1::__si_class_type_info *>(si->__base_type);
    }

    /* Cannot cast — return null. */
    return nullptr;
}
