#ifndef RBLUE_FLUTTER_ANCHOR_H
#define RBLUE_FLUTTER_ANCHOR_H

#ifdef __cplusplus
extern "C" {
#endif

/// Returns the address table used to root rblue's Dart FFI ABI.
const void *const *rblue_flutter_abi_anchor(void);

#ifdef __cplusplus
}
#endif

#endif /* RBLUE_FLUTTER_ANCHOR_H */
