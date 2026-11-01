// Vendored from https://github.com/rblue-rs/btleplug (src/droidplug/java) at
// rev 1af8299c291f24fca68dfb6b7ef1b5c0475892e1. BSD-3-Clause; see NOTICE.md in this directory for the license
// text and the vendoring rules. Do not edit by hand.

package io.github.gedgygedgy.rust.ops;

import java.io.Closeable;

final class FnRunnableImpl implements FnRunnable {
    private final FnAdapter<FnRunnableImpl, Void, Void, Void> adapter;

    private FnRunnableImpl(FnAdapter<FnRunnableImpl, Void, Void, Void> adapter) {
        this.adapter = adapter;
    }

    @Override
    public void run() {
        this.adapter.call(this, null, null);
    }

    @Override
    public void close() {
        this.adapter.close();
    }
}
