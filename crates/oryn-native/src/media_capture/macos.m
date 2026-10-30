// Parent-process AVFoundation adapter. Callback storage is one bounded latest
// sample; callers never receive Objective-C/device pointers or invoke selectors.
#import <AVFoundation/AVFoundation.h>
#import <CoreVideo/CoreVideo.h>
#import <CoreMedia/CoreMedia.h>
#import <AudioToolbox/AudioToolbox.h>
#include <math.h>
static const NSUInteger OrynCaptureLimit=16*1024*1024;
static char *OrynJSON(id value){NSData *bytes=[NSJSONSerialization dataWithJSONObject:value options:0 error:nil];if(!bytes||bytes.length>24*1024*1024)return NULL;char *copy=malloc(bytes.length+1);if(!copy)return NULL;memcpy(copy,bytes.bytes,bytes.length);copy[bytes.length]=0;return copy;}
void oryn_capture_free(char *value){free(value);}
static AVMediaType OrynType(int video){return video?AVMediaTypeVideo:AVMediaTypeAudio;}
char *oryn_capture_inventory(int video){@autoreleasepool{
 NSMutableArray *rows=[NSMutableArray array];
 // devicesWithMediaType is available across the supported macOS baseline and
 // includes external capture devices, unlike a built-in-only discovery list.
 for(AVCaptureDevice *device in [AVCaptureDevice devicesWithMediaType:OrynType(video)]){
  if(rows.count>=64)break;NSMutableArray *formats=[NSMutableArray array];NSUInteger index=0;
  for(AVCaptureDeviceFormat *format in device.formats){if(index>=512)break;NSMutableDictionary *row=[@{@"index":@(index++)} mutableCopy];
   if(video){CMVideoDimensions size=CMVideoFormatDescriptionGetDimensions(format.formatDescription);if(size.width<=0||size.height<=0||(uint64_t)size.width*size.height*4>OrynCaptureLimit)continue;row[@"width"]=@(size.width);row[@"height"]=@(size.height);NSMutableArray *rates=[NSMutableArray array];for(AVFrameRateRange *range in format.videoSupportedFrameRateRanges){[rates addObject:@[@(range.minFrameRate),@(range.maxFrameRate)]];if(rates.count>=32)break;}row[@"rates"]=rates;}
   else{const AudioStreamBasicDescription *audio=CMAudioFormatDescriptionGetStreamBasicDescription(format.formatDescription);if(!audio||audio->mSampleRate<=0||audio->mChannelsPerFrame==0||audio->mChannelsPerFrame>8)continue;row[@"sampleRate"]=@(audio->mSampleRate);row[@"channelCount"]=@(audio->mChannelsPerFrame);}
   [formats addObject:row];
  }
  [rows addObject:@{@"id":device.uniqueID,@"label":device.localizedName,@"formats":formats}];
 }
 return OrynJSON(@{@"authorization":@([AVCaptureDevice authorizationStatusForMediaType:OrynType(video)]),@"devices":rows});
}}
@interface OrynCapture : NSObject<AVCaptureVideoDataOutputSampleBufferDelegate,AVCaptureAudioDataOutputSampleBufferDelegate>
@property(nonatomic,strong) AVCaptureSession *session;
@property(nonatomic,strong) AVCaptureOutput *output;
@property(nonatomic,strong) NSLock *lock;
@property(nonatomic,strong) NSData *latest;
@property(nonatomic,strong) NSDictionary *metadata;
@property(nonatomic,strong) dispatch_queue_t queue;
@property(nonatomic) uint64_t sequence;
@property(nonatomic) uint64_t dropped;
@property(nonatomic) BOOL retired;
@property(nonatomic,strong) id failureObserver;
@property(nonatomic,strong) id disconnectObserver;
@property(nonatomic,strong) NSString *failure;
@end
@implementation OrynCapture
-(void)dealloc{if(_failureObserver)[[NSNotificationCenter defaultCenter] removeObserver:_failureObserver];if(_disconnectObserver)[[NSNotificationCenter defaultCenter] removeObserver:_disconnectObserver];}
-(void)captureOutput:(AVCaptureOutput *)output didOutputSampleBuffer:(CMSampleBufferRef)sample fromConnection:(AVCaptureConnection *)connection{
 @autoreleasepool{NSData *bytes=nil;NSDictionary *metadata=nil;double time=CMTimeGetSeconds(CMSampleBufferGetPresentationTimeStamp(sample));if(!isfinite(time)||time<0)return;
 CVPixelBufferRef pixel=CMSampleBufferGetImageBuffer(sample);
 if(pixel){size_t width=CVPixelBufferGetWidth(pixel),height=CVPixelBufferGetHeight(pixel);if(!width||!height||width>4096||height>4096||width*height*4>OrynCaptureLimit||CVPixelBufferGetPixelFormatType(pixel)!=kCVPixelFormatType_32BGRA)return;
  if(CVPixelBufferLockBaseAddress(pixel,kCVPixelBufferLock_ReadOnly)!=kCVReturnSuccess)return;size_t stride=CVPixelBufferGetBytesPerRow(pixel);const uint8_t *source=CVPixelBufferGetBaseAddress(pixel);NSMutableData *rgba=[NSMutableData dataWithLength:width*height*4];uint8_t *destination=rgba.mutableBytes;
  if(source&&stride>=width*4){for(size_t y=0;y<height;y++)for(size_t x=0;x<width;x++){const uint8_t *p=source+y*stride+x*4;uint8_t *d=destination+(y*width+x)*4;d[0]=p[2];d[1]=p[1];d[2]=p[0];d[3]=255;}bytes=rgba;metadata=@{@"kind":@"video",@"width":@(width),@"height":@(height),@"timestamp":@(time)};}
  CVPixelBufferUnlockBaseAddress(pixel,kCVPixelBufferLock_ReadOnly);
 }else{CMBlockBufferRef block=CMSampleBufferGetDataBuffer(sample);const AudioStreamBasicDescription *audio=CMAudioFormatDescriptionGetStreamBasicDescription(CMSampleBufferGetFormatDescription(sample));if(!block||!audio||audio->mFormatID!=kAudioFormatLinearPCM||!(audio->mFormatFlags&kAudioFormatFlagIsFloat)||(audio->mFormatFlags&kAudioFormatFlagIsNonInterleaved)||audio->mBitsPerChannel!=32)return;size_t count=CMBlockBufferGetDataLength(block);if(!count||count>OrynCaptureLimit||audio->mChannelsPerFrame==0||count%(audio->mChannelsPerFrame*4))return;NSMutableData *pcm=[NSMutableData dataWithLength:count];if(CMBlockBufferCopyDataBytes(block,0,count,pcm.mutableBytes)!=kCMBlockBufferNoErr)return;bytes=pcm;metadata=@{@"kind":@"audio",@"sampleRate":@(audio->mSampleRate),@"channels":@(audio->mChannelsPerFrame),@"timestamp":@(time)};}
 if(bytes){[self.lock lock];if(!self.retired){self.latest=bytes;self.metadata=metadata;self.sequence++;}[self.lock unlock];}
 }
}
-(void)captureOutput:(AVCaptureOutput *)output didDropSampleBuffer:(CMSampleBufferRef)sample fromConnection:(AVCaptureConnection *)connection{[self.lock lock];self.dropped++;[self.lock unlock];}
@end
// Returns a retained opaque capture on success; status: 1 permission, 2 missing,
// 3 unavailable/settings, 4 allocation. Permission prompting is the OS's own UI.
void *oryn_capture_open(int video,const char *identity,unsigned formatIndex,double rate,int *status){@autoreleasepool{@try{
 *status=3;AVAuthorizationStatus auth=[AVCaptureDevice authorizationStatusForMediaType:OrynType(video)];if(auth!=AVAuthorizationStatusAuthorized){if(auth==AVAuthorizationStatusNotDetermined&&[[NSBundle mainBundle] objectForInfoDictionaryKey:(video?@"NSCameraUsageDescription":@"NSMicrophoneUsageDescription")])[AVCaptureDevice requestAccessForMediaType:OrynType(video) completionHandler:^(BOOL granted){}];*status=1;return NULL;}
 AVCaptureDevice *device=[AVCaptureDevice deviceWithUniqueID:[NSString stringWithUTF8String:identity]];if(!device||![device hasMediaType:OrynType(video)]){*status=2;return NULL;}if(formatIndex>=device.formats.count)return NULL;
 NSError *error=nil;AVCaptureDeviceInput *input=[AVCaptureDeviceInput deviceInputWithDevice:device error:&error];if(!input)return NULL;
 OrynCapture *capture=[OrynCapture new];capture.lock=[NSLock new];capture.queue=dispatch_queue_create("org.oryn.capture.samples",DISPATCH_QUEUE_SERIAL);capture.session=[AVCaptureSession new];[capture.session beginConfiguration];
 if(![capture.session canAddInput:input])return NULL;[capture.session addInput:input];if(![device lockForConfiguration:&error])return NULL;@try{device.activeFormat=device.formats[formatIndex];
 if(video){BOOL valid=NO;for(AVFrameRateRange *range in device.activeFormat.videoSupportedFrameRateRanges)if(isfinite(rate)&&rate>=range.minFrameRate&&rate<=range.maxFrameRate){valid=YES;break;}if(!valid){return NULL;}device.activeVideoMinFrameDuration=CMTimeMakeWithSeconds(1./rate,1000000000);device.activeVideoMaxFrameDuration=device.activeVideoMinFrameDuration;}
 }@finally{[device unlockForConfiguration];}
 if(video){AVCaptureVideoDataOutput *output=[AVCaptureVideoDataOutput new];output.videoSettings=@{(NSString *)kCVPixelBufferPixelFormatTypeKey:@(kCVPixelFormatType_32BGRA)};output.alwaysDiscardsLateVideoFrames=YES;[output setSampleBufferDelegate:capture queue:capture.queue];capture.output=output;}
 else{AVCaptureAudioDataOutput *output=[AVCaptureAudioDataOutput new];const AudioStreamBasicDescription *selectedAudio=CMAudioFormatDescriptionGetStreamBasicDescription(device.activeFormat.formatDescription);if(!selectedAudio||!isfinite(selectedAudio->mSampleRate)||selectedAudio->mSampleRate<=0||!selectedAudio->mChannelsPerFrame||selectedAudio->mChannelsPerFrame>8)return NULL;output.audioSettings=@{AVFormatIDKey:@(kAudioFormatLinearPCM),AVLinearPCMIsFloatKey:@YES,AVLinearPCMBitDepthKey:@32,AVLinearPCMIsNonInterleaved:@NO,AVLinearPCMIsBigEndianKey:@NO,AVSampleRateKey:@(selectedAudio->mSampleRate),AVNumberOfChannelsKey:@(selectedAudio->mChannelsPerFrame)};[output setSampleBufferDelegate:capture queue:capture.queue];capture.output=output;}
 if(![capture.session canAddOutput:capture.output])return NULL;[capture.session addOutput:capture.output];[capture.session commitConfiguration];__weak OrynCapture *weakCapture=capture;
 capture.failureObserver=[[NSNotificationCenter defaultCenter] addObserverForName:AVCaptureSessionRuntimeErrorNotification object:capture.session queue:nil usingBlock:^(NSNotification *note){OrynCapture *strong=weakCapture;if(strong){[strong.lock lock];strong.failure=@"NotReadableError: native capture session failed";strong.retired=YES;strong.latest=nil;[strong.lock unlock];}}];
 capture.disconnectObserver=[[NSNotificationCenter defaultCenter] addObserverForName:AVCaptureDeviceWasDisconnectedNotification object:device queue:nil usingBlock:^(NSNotification *note){OrynCapture *strong=weakCapture;if(strong){[strong.lock lock];strong.failure=@"NotReadableError: native capture device disconnected";strong.retired=YES;strong.latest=nil;[strong.lock unlock];}}];
 [capture.session startRunning];if(!capture.session.running)return NULL;*status=0;return (__bridge_retained void *)capture;
}@catch(NSException *exception){*status=3;return NULL;}}}
char *oryn_capture_sample(void *opaque){@autoreleasepool{OrynCapture *capture=(__bridge OrynCapture *)opaque;[capture.lock lock];NSMutableDictionary *row=capture.metadata?[capture.metadata mutableCopy]:[NSMutableDictionary dictionary];row[@"ended"]=@(capture.retired||!capture.session.running);if(capture.failure)row[@"error"]=capture.failure;row[@"sequence"]=@(capture.sequence);row[@"dropped"]=@(capture.dropped);if(capture.latest){row[@"data"]=[capture.latest base64EncodedStringWithOptions:0];}[capture.lock unlock];return OrynJSON(row);}}
void oryn_capture_close(void *opaque){@autoreleasepool{OrynCapture *capture=(__bridge_transfer OrynCapture *)opaque;[capture.lock lock];capture.retired=YES;capture.latest=nil;[capture.lock unlock];if(capture.failureObserver)[[NSNotificationCenter defaultCenter] removeObserver:capture.failureObserver];if(capture.disconnectObserver)[[NSNotificationCenter defaultCenter] removeObserver:capture.disconnectObserver];if([capture.output isKindOfClass:[AVCaptureVideoDataOutput class]])[(AVCaptureVideoDataOutput *)capture.output setSampleBufferDelegate:nil queue:NULL];else[(AVCaptureAudioDataOutput *)capture.output setSampleBufferDelegate:nil queue:NULL];[capture.session stopRunning];dispatch_sync(capture.queue,^{});}}
@interface OrynAudioSink:NSObject
@property(nonatomic,strong) AVAudioEngine *engine;
@property(nonatomic,strong) AVAudioPlayerNode *player;
@property(nonatomic,strong) AVAudioFormat *format;
@property(nonatomic,strong) NSLock *lock;
@property(nonatomic) NSUInteger queued;
@end
@implementation OrynAudioSink
@end
void *oryn_audio_open(double rate,unsigned channels){@autoreleasepool{if(!isfinite(rate)||rate<8000||rate>192000||channels<1||channels>8)return NULL;OrynAudioSink *sink=[OrynAudioSink new];sink.lock=[NSLock new];sink.format=[[AVAudioFormat alloc] initWithCommonFormat:AVAudioPCMFormatFloat32 sampleRate:rate channels:channels interleaved:NO];sink.engine=[AVAudioEngine new];sink.player=[AVAudioPlayerNode new];[sink.engine attachNode:sink.player];[sink.engine connect:sink.player to:sink.engine.mainMixerNode format:sink.format];NSError *error=nil;if(![sink.engine startAndReturnError:&error])return NULL;[sink.player play];return (__bridge_retained void *)sink;}}
int oryn_audio_write(void *opaque,const uint8_t *bytes,size_t size,float volume){@autoreleasepool{OrynAudioSink *sink=(__bridge OrynAudioSink *)opaque;NSUInteger channels=sink.format.channelCount;if(!bytes||!size||size>1024*1024||size%(channels*4)||!isfinite(volume)||volume<0||volume>1)return 0;NSUInteger frames=size/(channels*4);[sink.lock lock];if(sink.queued>=8){[sink.lock unlock];return 2;}sink.queued++;[sink.lock unlock];AVAudioPCMBuffer *buffer=[[AVAudioPCMBuffer alloc] initWithPCMFormat:sink.format frameCapacity:(AVAudioFrameCount)frames];if(!buffer){[sink.lock lock];sink.queued--;[sink.lock unlock];return 0;}buffer.frameLength=(AVAudioFrameCount)frames;for(NSUInteger channel=0;channel<channels;channel++)for(NSUInteger frame=0;frame<frames;frame++){float sample;memcpy(&sample,bytes+(frame*channels+channel)*4,4);buffer.floatChannelData[channel][frame]=isfinite(sample)?sample:0;}sink.player.volume=volume;[sink.player scheduleBuffer:buffer completionHandler:^{[sink.lock lock];sink.queued--;[sink.lock unlock];}];return 1;}}
void oryn_audio_close(void *opaque){@autoreleasepool{OrynAudioSink *sink=(__bridge_transfer OrynAudioSink *)opaque;[sink.player stop];[sink.engine stop];}}
// 0 pending, 1 authorized, 2 denied/restricted, 3 missing usage declaration.
int oryn_capture_authorization(int video,int request){@autoreleasepool{AVAuthorizationStatus status=[AVCaptureDevice authorizationStatusForMediaType:OrynType(video)];if(status==AVAuthorizationStatusAuthorized)return 1;if(status==AVAuthorizationStatusDenied||status==AVAuthorizationStatusRestricted)return 2;if(request){if(![[NSBundle mainBundle] objectForInfoDictionaryKey:(video?@"NSCameraUsageDescription":@"NSMicrophoneUsageDescription")])return 3;[AVCaptureDevice requestAccessForMediaType:OrynType(video) completionHandler:^(BOOL granted){}];}return 0;}}
int oryn_capture_alive(void *opaque){@autoreleasepool{OrynCapture *capture=(__bridge OrynCapture *)opaque;[capture.lock lock];BOOL active=!capture.retired&&capture.session.running;[capture.lock unlock];return active?1:0;}}

// URL assets have no network access: the only URL is a private, exclusively
// created file containing the already-admitted response bytes. Every owner drop
// releases decoder references and removes the file.
#include <unistd.h>
@interface OrynURLMedia : NSObject
@property(nonatomic,strong) NSURL *url;
@property(nonatomic,strong) AVURLAsset *asset;
@property(nonatomic,strong) AVAssetImageGenerator *images;
@property(nonatomic) double duration;
@property(nonatomic) uint32_t width;
@property(nonatomic) uint32_t height;
@end
@implementation OrynURLMedia
-(void)dealloc{[_images cancelAllCGImageGeneration];if(_url)[[NSFileManager defaultManager] removeItemAtURL:_url error:nil];}
@end
// AVFoundation's local-file resolver requires a container hint for some valid
// media (notably WAV). Derive it from admitted bytes, never an author URL/MIME.
// This is only a resolver hint: AVFoundation must still validate the asset.
static NSString *OrynURLContainerSuffix(const uint8_t *bytes,size_t length) {
 if(length>=12&&!memcmp(bytes,"RIFF",4)&&!memcmp(bytes+8,"WAVE",4))return @".wav";
 if(length>=12&&!memcmp(bytes,"FORM",4)&&(!memcmp(bytes+8,"AIFF",4)||!memcmp(bytes+8,"AIFC",4)))return @".aiff";
 if(length>=12&&!memcmp(bytes+4,"ftyp",4))return @".mp4";
 if(length>=8&&!memcmp(bytes,"caff",4))return @".caf";
 return @"";
}
void *oryn_url_media_open(const uint8_t *bytes,size_t length){@autoreleasepool{
 if(!bytes||!length||length>16*1024*1024)return NULL;
 NSString *suffix=OrynURLContainerSuffix(bytes,length);
 NSString *template=[NSTemporaryDirectory() stringByAppendingPathComponent:[@"oryn-media-XXXXXX" stringByAppendingString:suffix]];
 char *path=strdup(template.fileSystemRepresentation);if(!path)return NULL;int fd=mkstemps(path,(int)suffix.length);if(fd<0){free(path);return NULL;}
 size_t offset=0;while(offset<length){ssize_t count=write(fd,bytes+offset,length-offset);if(count<=0){close(fd);unlink(path);free(path);return NULL;}offset+=(size_t)count;}close(fd);
 OrynURLMedia *value=[OrynURLMedia new];value.url=[NSURL fileURLWithPath:@(path)];free(path);
 value.asset=[AVURLAsset URLAssetWithURL:value.url options:@{AVURLAssetPreferPreciseDurationAndTimingKey:@YES,AVURLAssetReferenceRestrictionsKey:@(AVAssetReferenceRestrictionForbidAll)}];
 double duration=CMTimeGetSeconds(value.asset.duration);if(!isfinite(duration)||duration<0||!value.asset.playable)return NULL;value.duration=duration;
 AVAssetTrack *video=[value.asset tracksWithMediaType:AVMediaTypeVideo].firstObject;
 AVAssetTrack *audio=[value.asset tracksWithMediaType:AVMediaTypeAudio].firstObject;if(!video&&!audio)return NULL;
 if(video){CGSize size=CGSizeApplyAffineTransform(video.naturalSize,video.preferredTransform);double width=fabs(size.width),height=fabs(size.height);if(width<1||height<1||width>8192||height>8192||width*height*4>OrynCaptureLimit)return NULL;value.width=(uint32_t)width;value.height=(uint32_t)height;value.images=[AVAssetImageGenerator assetImageGeneratorWithAsset:value.asset];value.images.appliesPreferredTrackTransform=YES;value.images.requestedTimeToleranceBefore=kCMTimeZero;value.images.requestedTimeToleranceAfter=CMTimeMake(1,30);}
 return (__bridge_retained void *)value;
}}
static NSDictionary *OrynURLAudioFormat(OrynURLMedia *value) {
 AVAssetTrack *track=[value.asset tracksWithMediaType:AVMediaTypeAudio].firstObject;
 id description=track.formatDescriptions.firstObject;
 const AudioStreamBasicDescription *format=description?CMAudioFormatDescriptionGetStreamBasicDescription((__bridge CMAudioFormatDescriptionRef)description):NULL;
 return @{@"sampleRate":@(format&&isfinite(format->mSampleRate)?format->mSampleRate:0),@"channels":@(format?format->mChannelsPerFrame:0)};
}
char *oryn_url_media_metadata(void *raw){@autoreleasepool{OrynURLMedia *value=(__bridge OrynURLMedia *)raw;if(!value)return NULL;NSMutableDictionary *row=[@{@"duration":@(value.duration),@"width":@(value.width),@"height":@(value.height),@"audio":([value.asset tracksWithMediaType:AVMediaTypeAudio].count>0?@YES:@NO)} mutableCopy];[row addEntriesFromDictionary:OrynURLAudioFormat(value)];return OrynJSON(row);}}
static NSDictionary *OrynURLAudioWindow(OrynURLMedia *value,double start,double until) {
 if(!value||!isfinite(start)||start<0||!isfinite(until)||until<start||until-start>0.3)return nil;
 AVAssetTrack *track=[value.asset tracksWithMediaType:AVMediaTypeAudio].firstObject;
 if(!track||until==start||start>=value.duration)return @{};
 NSError *error=nil;
 AVAssetReader *reader=[[AVAssetReader alloc] initWithAsset:value.asset error:&error];
 if(!reader)return nil;
 NSDictionary *settings=@{AVFormatIDKey:@(kAudioFormatLinearPCM),AVLinearPCMBitDepthKey:@32,AVLinearPCMIsFloatKey:@YES,AVLinearPCMIsBigEndianKey:@NO,AVLinearPCMIsNonInterleaved:@NO};
 AVAssetReaderTrackOutput *output=[[AVAssetReaderTrackOutput alloc] initWithTrack:track outputSettings:settings];
 if(!output||![reader canAddOutput:output])return nil;
 [reader addOutput:output];
 reader.timeRange=CMTimeRangeMake(CMTimeMakeWithSeconds(start,60000),CMTimeMakeWithSeconds(MIN(until,value.duration)-start,60000));
 if(![reader startReading])return nil;
 NSMutableData *audio=[NSMutableData data];double rate=0;uint32_t channels=0;
 if(!audio){[reader cancelReading];return nil;}
 while(reader.status==AVAssetReaderStatusReading){
  CMSampleBufferRef sample=[output copyNextSampleBuffer];if(!sample)break;
  const AudioStreamBasicDescription *format=CMAudioFormatDescriptionGetStreamBasicDescription(CMSampleBufferGetFormatDescription(sample));
  CMBlockBufferRef block=CMSampleBufferGetDataBuffer(sample);size_t length=block?CMBlockBufferGetDataLength(block):0;
  if(!format||!isfinite(format->mSampleRate)||format->mSampleRate<8000||format->mSampleRate>192000||format->mChannelsPerFrame<1||format->mChannelsPerFrame>8||format->mFormatID!=kAudioFormatLinearPCM||format->mBitsPerChannel!=32||!(format->mFormatFlags&kAudioFormatFlagIsFloat)||length%(format->mChannelsPerFrame*4)||(channels&&(channels!=format->mChannelsPerFrame||rate!=format->mSampleRate))){CFRelease(sample);[reader cancelReading];return nil;}
  rate=format->mSampleRate;channels=format->mChannelsPerFrame;
  // Readers may return whole codec packets overlapping the requested interval.
  // Trim their PCM frames by native timestamps before allocating transfer bytes.
  double timestamp=CMTimeGetSeconds(CMSampleBufferGetPresentationTimeStamp(sample));
  if(!isfinite(timestamp)){CFRelease(sample);[reader cancelReading];return nil;}
  size_t frameBytes=channels*4,frames=length/frameBytes;
  size_t first=(size_t)MIN((double)frames,MAX(0,ceil((start-timestamp)*rate)));
  size_t last=(size_t)MIN((double)frames,MAX(0,ceil((MIN(until,value.duration)-timestamp)*rate)));
  if(last<=first){CFRelease(sample);continue;}
  size_t count=(last-first)*frameBytes;
  if(count>1024*1024-audio.length){CFRelease(sample);[reader cancelReading];return nil;}
  size_t old=audio.length;[audio increaseLengthBy:count];
  if(!audio.mutableBytes){CFRelease(sample);[reader cancelReading];return nil;}
  OSStatus status=CMBlockBufferCopyDataBytes(block,first*frameBytes,count,(uint8_t *)audio.mutableBytes+old);
  CFRelease(sample);if(status){[reader cancelReading];return nil;}
 }
 if(reader.status==AVAssetReaderStatusFailed)return nil;
 if(!audio.length)return @{};
 NSString *encoded=[audio base64EncodedStringWithOptions:0];if(!encoded)return nil;
 return @{@"audio":encoded,@"sampleRate":@(rate),@"channels":@(channels)};
}
char *oryn_url_media_audio(void *raw,double start,double until){@autoreleasepool{@try{
 NSDictionary *row=OrynURLAudioWindow((__bridge OrynURLMedia *)raw,start,until);
 return row?OrynJSON(row):NULL;
}@catch(NSException *exception){return NULL;}}}
char *oryn_url_media_sample(void *raw,double position,double audioStart,double audioUntil){@autoreleasepool{
 OrynURLMedia *value=(__bridge OrynURLMedia *)raw;if(!value||!isfinite(position)||position<0||!isfinite(audioStart)||audioStart<0||!isfinite(audioUntil)||audioUntil<audioStart||audioUntil-audioStart>0.3)return NULL;
 NSMutableDictionary *result=[@{@"timestamp":@(position),@"ended":(position>=value.duration?@YES:@NO)} mutableCopy];
 if(value.images&&value.duration>0){CMTime actual;NSError *error=nil;double at=MIN(position,MAX(0,value.duration-1.0/600));CGImageRef image=[value.images copyCGImageAtTime:CMTimeMakeWithSeconds(at,60000) actualTime:&actual error:&error];if(!image)return NULL;size_t width=CGImageGetWidth(image),height=CGImageGetHeight(image);if(!width||!height||width>8192||height>8192||width*height*4>OrynCaptureLimit){CGImageRelease(image);return NULL;}NSMutableData *rgba=[NSMutableData dataWithLength:width*height*4];CGColorSpaceRef colors=CGColorSpaceCreateDeviceRGB();CGContextRef context=CGBitmapContextCreate(rgba.mutableBytes,width,height,8,width*4,colors,kCGImageAlphaPremultipliedLast|kCGBitmapByteOrder32Big);CGColorSpaceRelease(colors);if(!context){CGImageRelease(image);return NULL;}CGContextDrawImage(context,CGRectMake(0,0,width,height),image);CGContextRelease(context);CGImageRelease(image);result[@"width"]=@(width);result[@"height"]=@(height);result[@"data"]=[rgba base64EncodedStringWithOptions:0];}
 NSDictionary *audio=OrynURLAudioWindow(value,audioStart,audioUntil);if(!audio)return NULL;
 [result addEntriesFromDictionary:audio];
 return OrynJSON(result);
}}
void oryn_url_media_close(void *raw){@autoreleasepool{OrynURLMedia *value=(__bridge_transfer OrynURLMedia *)raw;[value.images cancelAllCGImageGeneration];}}

int oryn_url_media_can_play(const char *mime){@autoreleasepool{if(!mime)return 0;NSString *value=[NSString stringWithUTF8String:mime];return value&&[AVURLAsset isPlayableExtendedMIMEType:value];}}

#include "webm_macos.inc"
#include "rate_sink_macos.inc"
