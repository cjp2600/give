#import <AppKit/AppKit.h>

// Read actual file URLs before image representations: a copied image file must
// keep its original name and bytes rather than becoming a newly encoded image.
void give_catalog_clipboard_files(void *context,
                                 void (*append)(void *, const char *)) {
    @autoreleasepool {
        NSPasteboard *pasteboard = NSPasteboard.generalPasteboard;
        NSArray<NSURL *> *urls = [pasteboard readObjectsForClasses:@[NSURL.class]
            options:@{NSPasteboardURLReadingFileURLsOnlyKey: @YES}];
        for (NSURL *url in urls) {
            if (url.fileURL) append(context, url.fileSystemRepresentation);
        }
    }
}

void give_catalog_clipboard_image(void *context,
    void (*append)(void *, const char *, const unsigned char *, size_t)) {
    @autoreleasepool {
        NSPasteboard *pasteboard = NSPasteboard.generalPasteboard;
        NSArray<NSString *> *types = @[@"public.png", @"public.jpeg",
            @"org.webmproject.webp", @"com.compuserve.gif", @"public.svg-image",
            @"com.microsoft.bmp", @"public.tiff"];
        NSArray<NSString *> *extensions = @[@"png", @"jpg", @"webp", @"gif",
            @"svg", @"bmp", @"tiff"];
        for (NSUInteger i = 0; i < types.count; ++i) {
            NSData *data = [pasteboard dataForType:types[i]];
            if (data.length) {
                append(context, extensions[i].UTF8String, data.bytes, data.length);
                return;
            }
        }
    }
}

bool give_catalog_trash(const char *path, void *context,
                       void (*error_message)(void *, const char *)) {
    @autoreleasepool {
        NSURL *url = [NSURL fileURLWithFileSystemRepresentation:path
            isDirectory:NO relativeToURL:nil];
        NSError *error = nil;
        BOOL success = [NSFileManager.defaultManager trashItemAtURL:url
            resultingItemURL:nil error:&error];
        if (!success) error_message(context, error.localizedDescription.UTF8String);
        return success;
    }
}
