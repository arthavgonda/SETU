#include <QWindow>

#import <AppKit/AppKit.h>

void setuRaiseOverMenuBar(QWindow *window)
{
    if (!window)
        return;
    NSView *view = (__bridge NSView *)reinterpret_cast<void *>(window->winId());
    NSWindow *w = view.window;
    if (!w)
        return;
    [w setLevel:NSStatusWindowLevel + 1];
    [w setCollectionBehavior:NSWindowCollectionBehaviorCanJoinAllSpaces | NSWindowCollectionBehaviorStationary | NSWindowCollectionBehaviorFullScreenAuxiliary | NSWindowCollectionBehaviorIgnoresCycle];
    [w setHasShadow:NO];
    [w setHidesOnDeactivate:NO];
}
