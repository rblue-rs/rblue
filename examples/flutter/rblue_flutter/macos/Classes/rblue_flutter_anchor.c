#include <rblue.h>
#include "rblue_flutter_anchor.h"

// Dart FFI looks these up dynamically, so ordinary source references do not
// otherwise exist. Keep a relocation to every entry point in one object file.
// Package.swift also roots the names at the final application link so that
// dead stripping cannot discard them after this package has been archived.
__attribute__((used))
static const void *const rblue_flutter_abi_roots[] = {
    (const void *)&rblue_new,
    (const void *)&rblue_free,
    (const void *)&rblue_request,
    (const void *)&rblue_shutdown,
    (const void *)&rblue_set_event_callback,
    (const void *)&rblue_set_owned_response_callback,
    (const void *)&rblue_set_response_callback,
    (const void *)&rblue_free_string,
    (const void *)&rblue_free_payload,
};

const void *const *rblue_flutter_abi_anchor(void) {
    return rblue_flutter_abi_roots;
}
