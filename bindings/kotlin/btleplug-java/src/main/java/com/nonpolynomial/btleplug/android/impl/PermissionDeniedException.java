// Vendored from https://github.com/rblue-rs/btleplug (src/droidplug/java) at
// rev 1af8299c291f24fca68dfb6b7ef1b5c0475892e1. BSD-3-Clause; see NOTICE.md in this directory for the license
// text and the vendoring rules. Do not edit by hand.

package com.nonpolynomial.btleplug.android.impl;

class PermissionDeniedException extends BluetoothException {
    public PermissionDeniedException() {
        super();
    }

    public PermissionDeniedException(Throwable cause) {
        super(cause);
    }
}
