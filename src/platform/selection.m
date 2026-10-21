// Native, read-only text elements for selection readers and macOS Services.
// GPUI 0.2.2 renders these areas itself and has no accessibility text nodes.
#import <AppKit/AppKit.h>
#import <objc/runtime.h>
#import <objc/message.h>

@interface GiveTextElement : NSAccessibilityElement
@property(nonatomic, weak) NSView *owner;
@property(nonatomic, copy) NSString *text;
@property(nonatomic, copy) NSString *selection;
@property(nonatomic) NSRange selectionRange;
@property(nonatomic) BOOL focused;
@property(nonatomic) NSRect localFrame;
@end
@implementation GiveTextElement
- (NSString *)accessibilityRole { return NSAccessibilityTextAreaRole; }
- (id)accessibilityValue { return self.text ?: @""; }
- (NSString *)accessibilitySelectedText { return self.selection ?: @""; }
- (NSRange)accessibilitySelectedTextRange { return self.selectionRange; }
- (NSArray *)accessibilitySelectedTextRanges { return @[[NSValue valueWithRange:self.selectionRange]]; }
- (NSInteger)accessibilityNumberOfCharacters { return self.text.length; }
- (NSRange)accessibilityVisibleCharacterRange { return NSMakeRange(0, self.text.length); }
- (BOOL)isAccessibilityFocused { return self.focused; }
- (BOOL)isAccessibilityEnabled { return YES; }
- (id)accessibilityParent { return self.owner.window; }
- (id)accessibilityWindow { return self.owner.window; }
- (id)accessibilityTopLevelUIElement { return self.owner.window; }
- (NSRect)accessibilityFrame {
    NSView *view = self.owner;
    NSRect rect = self.localFrame;
    if (!view.isFlipped) rect.origin.y = view.bounds.size.height - NSMaxY(rect);
    return [view.window convertRectToScreen:[view convertRect:rect toView:nil]];
}
- (NSString *)accessibilityStringForRange:(NSRange)range {
    if (range.location > self.text.length || range.length > self.text.length - range.location) return nil;
    return [self.text substringWithRange:range];
}
- (NSAttributedString *)accessibilityAttributedStringForRange:(NSRange)range {
    NSString *text = [self accessibilityStringForRange:range];
    return text ? [[NSAttributedString alloc] initWithString:text] : nil;
}
- (BOOL)isAccessibilitySelectorAllowed:(SEL)selector {
    if (selector == @selector(setAccessibilityValue:) || selector == @selector(setAccessibilitySelectedText:) || selector == @selector(setAccessibilitySelectedTextRange:) || selector == @selector(setAccessibilitySelectedTextRanges:)) return NO;
    return [super isAccessibilitySelectorAllowed:selector];
}
@end

static char nodesKey;
static char viewKey;
static char childrenKey;
static GiveTextElement *activeElement(NSView *view) {
    for (GiveTextElement *node in objc_getAssociatedObject(view, &nodesKey)) {
        if (node.focused) return node;
    }
    return nil;
}
static id focusedElement(id object, SEL selector) {
    NSView *view = objc_getAssociatedObject(NSApp.keyWindow, &viewKey);
    GiveTextElement *node = activeElement(view);
    if (node) return node;
    struct objc_super parent = { object, NSApplication.class };
    return ((id (*)(struct objc_super *, SEL))objc_msgSendSuper)(&parent, selector);
}
static id requestor(id object, SEL selector, NSString *sendType, NSString *returnType) {
    GiveTextElement *node = activeElement(object);
    if (node.selection.length && [sendType isEqualToString:NSPasteboardTypeString] && !returnType) return object;
    struct objc_super parent = { object, class_getSuperclass(object_getClass(object)) };
    return ((id (*)(struct objc_super *, SEL, id, id))objc_msgSendSuper)(&parent, selector, sendType, returnType);
}
static BOOL writeSelection(id object, SEL selector, NSPasteboard *pasteboard, NSArray *types) {
    (void)selector;
    GiveTextElement *node = activeElement(object);
    if (!node.selection.length || ![types containsObject:NSPasteboardTypeString]) return NO;
    [pasteboard clearContents];
    return [pasteboard setString:node.selection forType:NSPasteboardTypeString];
}
static void copySelection(id object, SEL selector, id sender) {
    (void)selector; (void)sender;
    GiveTextElement *node = activeElement(object);
    if (!node.selection.length) return;
    NSPasteboard *pasteboard = NSPasteboard.generalPasteboard;
    [pasteboard clearContents];
    [pasteboard setString:node.selection forType:NSPasteboardTypeString];
}

// The NSView belongs to GPUI. Associated objects own nodes, and nodes hold weak
// view references, so closing a window releases the entire bridge.
void give_selection_update(void *rawView, unsigned index, const char *label,
                           const char *text, const char *selection,
                           size_t start, size_t length, bool focused, bool visible,
                           double x, double y, double width, double height) {
    NSView *view = (__bridge NSView *)rawView;
    NSMutableArray *nodes = objc_getAssociatedObject(view, &nodesKey);
    if (!nodes) {
        nodes = [NSMutableArray array];
        for (unsigned n = 0; n < 3; ++n) {
            GiveTextElement *node = [GiveTextElement new];
            node.owner = view; node.text = @""; node.selection = @"";
            node.accessibilityElement = YES;
            [nodes addObject:node];
        }
        objc_setAssociatedObject(view, &nodesKey, nodes, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
        objc_setAssociatedObject(view.window, &viewKey, view, OBJC_ASSOCIATION_ASSIGN);
        objc_setAssociatedObject(view.window, &childrenKey, view.window.accessibilityChildren ?: @[], OBJC_ASSOCIATION_RETAIN_NONATOMIC);
        // Add only to this app's GPUI classes, preserving AppKit's
        // inherited implementation when a text pane doesn't have focus.
        class_addMethod(object_getClass(NSApp), @selector(accessibilityFocusedUIElement), (IMP)focusedElement, "@@:");
        class_addMethod(object_getClass(view), @selector(validRequestorForSendType:returnType:), (IMP)requestor, "@@:@@");
        class_addMethod(object_getClass(view), @selector(writeSelectionToPasteboard:types:), (IMP)writeSelection, "B@:@@");
        class_addMethod(object_getClass(view), @selector(copy:), (IMP)copySelection, "v@:@");
    }
    if (index >= nodes.count) return;
    GiveTextElement *node = nodes[index];
    NSString *newSelection = selection ? @(selection) : node.selection;
    BOOL selectionChanged = ![node.selection isEqualToString:newSelection];
    BOOL focusChanged = node.focused != focused;
    node.accessibilityLabel = @(label);
    if (text) node.text = @(text);
    node.selection = newSelection;
    node.selectionRange = NSMakeRange(MIN(start, node.text.length), MIN(length, node.text.length - MIN(start, node.text.length)));
    if (focused) for (GiveTextElement *other in nodes) if (other != node) other.focused = NO;
    node.focused = focused;
    node.accessibilityHidden = !visible;
    node.localFrame = NSMakeRect(x, y, width, height);
    NSArray *visibleNodes = [nodes filteredArrayUsingPredicate:[NSPredicate predicateWithBlock:^BOOL(GiveTextElement *element, NSDictionary *bindings) { (void)bindings; return !element.accessibilityHidden; }]];
    view.window.accessibilityChildren = [objc_getAssociatedObject(view.window, &childrenKey) arrayByAddingObjectsFromArray:visibleNodes];
    if (focusChanged && focused) NSAccessibilityPostNotification(node, NSAccessibilityFocusedUIElementChangedNotification);
    if (selectionChanged) NSAccessibilityPostNotification(node, NSAccessibilitySelectedTextChangedNotification);
}
