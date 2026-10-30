// Explicit VP9/Opus decoder for WebM packets demuxed by webm.rs. This adapter
// has no URL API. Container byte bounds and track selection precede admission.
#import <VideoToolbox/VideoToolbox.h>
#import <dispatch/dispatch.h>
#include <vpx/vpx_decoder.h>
#include <vpx/vp8dx.h>
#include <limits.h>
#include <stdlib.h>
#include <string.h>
#include <vpx/vpx_frame_buffer.h>
@interface OrynWebM : NSObject {
@public VTDecompressionSessionRef decoder;
@public CMVideoFormatDescriptionRef format;
@public CVPixelBufferRef image;
@public OSStatus decodeError;
@public vpx_codec_ctx_t vp8;
@public BOOL vp8Ready;
@public BOOL isVP8;
@public BOOL softwarePreferred;
@public OSStatus nativeStatus;
@public vpx_codec_err_t softwareStatus;
@public void *framePool;
@public unsigned testMode;
@public BOOL packetFailureInjected;
}
@property(nonatomic,strong) NSData *bytes;
@property(nonatomic,strong) NSArray *video;
@property(nonatomic,strong) NSData *pcm;
@property(nonatomic) NSUInteger cursor;
@property(nonatomic) double lastPosition;
@property(nonatomic) double duration;
@property(nonatomic) uint32_t width;
@property(nonatomic) uint32_t height;
@property(nonatomic) uint32_t channels;
@property(nonatomic) NSUInteger manifestBytes;
@end
@implementation OrynWebM
-(void)dealloc{if(vp8Ready)vpx_codec_destroy(&vp8);if(decoder){VTDecompressionSessionInvalidate(decoder);CFRelease(decoder);}if(format)CFRelease(format);if(image)CVPixelBufferRelease(image);if(framePool)free(framePool);}
@end
static void OrynVP9Output(void *ref,void *source,OSStatus status,VTDecodeInfoFlags flags,CVImageBufferRef buffer,CMTime pts,CMTime duration){
 OrynWebM *media=(__bridge OrynWebM *)ref;if(status){media->decodeError=status;return;}if(buffer){if(media->image)CVPixelBufferRelease(media->image);media->image=CVPixelBufferRetain(buffer);}
}
#include "webm_vpx.inc"
static BOOL OrynVP9Session(OrynWebM *media){
 if(media->isVP8||media->softwarePreferred||media->testMode==1||media->testMode==3||media->testMode==4)return OrynSoftwareSession(media);
 OrynVideoReset(media);
 static dispatch_once_t vp9Registration;
 dispatch_once(&vp9Registration, ^{if (@available(macOS 11.0, *)) VTRegisterSupplementalVideoDecoderIfAvailable(kCMVideoCodecType_VP9);});
 VTDecompressionOutputCallbackRecord callback={OrynVP9Output,(__bridge void *)media};
 NSDictionary *attributes=@{(__bridge NSString *)kCVPixelBufferPixelFormatTypeKey:@(kCVPixelFormatType_32BGRA),(__bridge NSString *)kCVPixelBufferWidthKey:@(media.width),(__bridge NSString *)kCVPixelBufferHeightKey:@(media.height)};
 if(media->testMode==2)media->nativeStatus=kVTVideoDecoderNotAvailableNowErr;
 else if(media->format)media->nativeStatus=VTDecompressionSessionCreate(kCFAllocatorDefault,media->format,NULL,(__bridge CFDictionaryRef)attributes,&callback,&media->decoder);
 // A missing format carries the original CoreMedia failure saved by open/probe.
 if(media->decoder&&media->nativeStatus==noErr)return YES;
 media->softwarePreferred=YES;return OrynSoftwareSession(media);
}
typedef struct {const uint8_t *bytes;UInt32 length;BOOL supplied;AudioStreamPacketDescription description;} OrynOpusInput;
// A packet boundary is temporary starvation, not permanent stream EOF. Returning
// noErr with zero packets latches EOF in AudioConverter and drops later packets.
static const OSStatus OrynOpusAwaitPacket=2002872692; // 'wait', callback-owned
static UInt32 OrynOpusFrames(const uint8_t *bytes,UInt32 length){if(!length)return 0;uint8_t toc=bytes[0],config=toc>>3;UInt32 frames=config>=16?(120u<<(config&3)):config>=12?(480u<<(config&1)):(UInt32[]){480,960,1920,2880}[config&3];UInt32 count=(toc&3)==0?1:(toc&3)==3?(length>1?bytes[1]&63:0):2;return count&&frames*count<=5760?frames*count:0;}
static OSStatus OrynOpusRead(AudioConverterRef converter,UInt32 *packets,AudioBufferList *data,AudioStreamPacketDescription **descriptions,void *raw){
 OrynOpusInput *input=raw;if(input->supplied){*packets=0;return OrynOpusAwaitPacket;}input->supplied=YES;*packets=1;data->mNumberBuffers=1;data->mBuffers[0].mNumberChannels=0;data->mBuffers[0].mData=(void *)input->bytes;data->mBuffers[0].mDataByteSize=input->length;input->description=(AudioStreamPacketDescription){0,OrynOpusFrames(input->bytes,input->length),input->length};if(descriptions)*descriptions=&input->description;return noErr;
}
static NSData *OrynDecodeOpus(NSData *bytes,NSDictionary *track,double duration,OSStatus *failure){
 if(failure)*failure=kAudio_ParamError;
 UInt32 channels=[track[@"channels"] unsignedIntValue];if(!channels||channels>2||!isfinite(duration)||duration<=0||duration*48000*channels*4>64*1024*1024)return nil;
 AudioStreamBasicDescription input={0};input.mSampleRate=48000;input.mFormatID=kAudioFormatOpus;input.mChannelsPerFrame=channels;
 AudioStreamBasicDescription output={0};output.mSampleRate=48000;output.mFormatID=kAudioFormatLinearPCM;output.mFormatFlags=kAudioFormatFlagIsFloat|kAudioFormatFlagIsPacked;output.mBytesPerPacket=channels*4;output.mFramesPerPacket=1;output.mBytesPerFrame=channels*4;output.mChannelsPerFrame=channels;output.mBitsPerChannel=32;
 AudioConverterRef converter=NULL;OSStatus creation=AudioConverterNew(&input,&output,&converter);if(failure)*failure=creation;if(creation!=noErr)return nil;
 // WebM mapping family zero is mono/stereo. The Opus pre-skip and output gain
 // are applied below in timestamp space, rather than accidentally twice through
 // a container-specific magic cookie. Multistream mapping is rejected by demux.
 NSArray *private=track[@"private"];int16_t gainBits=(int16_t)([private[16] unsignedIntValue]|([private[17] unsignedIntValue]<<8));float gain=powf(10.f,(float)gainBits/(256.f*20.f));
 NSMutableData *pcm=[NSMutableData dataWithLength:(NSUInteger)ceil(duration*48000)*channels*4];BOOL valid=pcm&&pcm.mutableBytes;NSUInteger decodedFrames=0;BOOL initial=YES;
 if(!valid){AudioConverterDispose(converter);if(failure)*failure=kAudio_ParamError;return nil;}
 for(NSDictionary *packet in track[@"packets"]){NSUInteger offset=[packet[@"offset"] unsignedIntegerValue],length=[packet[@"length"] unsignedIntegerValue];if(!length||offset>bytes.length||length>bytes.length-offset||length>UINT32_MAX){valid=NO;break;}
  OrynOpusInput supplied={(const uint8_t *)bytes.bytes+offset,(UInt32)length,NO,{0}};
  // Opus packets encode at most 120 ms. One compressed packet can produce
  // several output chunks; retain decoder state across packets for prediction.
  float decoded[5760*2];AudioBufferList buffers={1,{{channels,sizeof(decoded),decoded}}};UInt32 frames=5760;
  OSStatus status=AudioConverterFillComplexBuffer(converter,OrynOpusRead,&supplied,&frames,&buffers,NULL);
  UInt32 expected=OrynOpusFrames(supplied.bytes,supplied.length);
  if((status!=noErr&&status!=OrynOpusAwaitPacket)||!frames||!expected||frames>expected||(!initial&&frames!=expected)||buffers.mBuffers[0].mDataByteSize!=frames*channels*4){if(failure)*failure=status!=noErr&&status!=OrynOpusAwaitPacket?status:kAudio_ParamError;valid=NO;break;}
  // Account for the converter's initial trimmed frames from actual output,
  // rather than guessing a platform delay or applying WebM pre-skip twice.
  NSUInteger trimmed=expected-frames;initial=NO;decodedFrames+=frames;
  double timestamp=[packet[@"time"] doubleValue],discard=[packet[@"discard"] doubleValue];
  if(!isfinite(timestamp)||!isfinite(discard)||fabs(timestamp)>duration+0.12||fabs(discard)*48000>expected){if(failure)*failure=kAudio_ParamError;valid=NO;break;}
  int64_t base=llround(timestamp*48000)+(int64_t)trimmed;
  NSUInteger head=discard<0?(NSUInteger)ceil(-discard*48000):0;
  NSUInteger first=head>trimmed?head-trimmed:0,last=frames;
  NSUInteger tail=discard>0?(NSUInteger)ceil(discard*48000):0;
  last=tail>=last?0:last-tail;
  float *target=pcm.mutableBytes;NSUInteger total=pcm.length/(channels*4);
  for(NSUInteger frame=first;frame<last;frame++){int64_t at=base+(int64_t)frame;if(at<0)continue;if((uint64_t)at>=total)break;for(UInt32 c=0;c<channels;c++)target[at*channels+c]=decoded[frame*channels+c]*gain;}
 }
 AudioConverterDispose(converter);return valid&&decodedFrames?pcm:nil;
}
int oryn_webm_opus_unit_checks(void){@autoreleasepool{
 uint8_t silence[]={0xf8,0xff,0xfe,0xf8,0xff,0xfe,0xf8,0xff,0xfe};
 NSData *bytes=[NSData dataWithBytes:silence length:sizeof(silence)];
 NSMutableArray *packets=[NSMutableArray array];for(unsigned i=0;i<3;i++)[packets addObject:@{@"offset":@(i*3),@"length":@3,@"time":@(i*0.02),@"discard":@(i==2?0.001:0)}];
 NSMutableArray *private=[NSMutableArray array];for(unsigned i=0;i<19;i++)[private addObject:@0];
 NSDictionary *track=@{@"channels":@1,@"private":private,@"packets":packets};OSStatus status=noErr;
 NSData *pcm=OrynDecodeOpus(bytes,track,0.06,&status);
 return pcm&&pcm.length==2880*4&&status==noErr;
}}
static void *OrynWebMOpen(const uint8_t *bytes,size_t length,const uint8_t *manifest,size_t manifestLength,unsigned testMode,char **diagnostic){@autoreleasepool{
 if(diagnostic)*diagnostic=NULL;
 if(!bytes||!length||length>16*1024*1024||!manifest||manifestLength>32*1024*1024)return NULL;
 NSDictionary *description=[NSJSONSerialization JSONObjectWithData:[NSData dataWithBytes:manifest length:manifestLength] options:0 error:nil];if(![description isKindOfClass:NSDictionary.class])return NULL;
 OrynWebM *media=[OrynWebM new];media->testMode=testMode;media.bytes=[NSData dataWithBytes:bytes length:length];media.manifestBytes=manifestLength*2;media.duration=[description[@"duration"] doubleValue];media.lastPosition=-1;
 for(NSDictionary *track in description[@"tracks"]){if([track[@"codec"] isEqual:@"A_OPUS"]){media.channels=[track[@"channels"] unsignedIntValue];OSStatus status=noErr;media.pcm=OrynDecodeOpus(media.bytes,track,media.duration,&status);if(!media.pcm){if(diagnostic)*diagnostic=OrynJSON(@{@"stage":@"opus-init",@"audioStatus":@(status),@"mode":@(testMode)});return NULL;}}
  else if([track[@"codec"] isEqual:@"V_VP8"]){media.width=[track[@"width"] unsignedIntValue];media.height=[track[@"height"] unsignedIntValue];media.video=track[@"packets"];media->isVP8=YES;if(!media.width||!media.height||(uint64_t)media.width*media.height*4>OrynCaptureLimit||!OrynVP9Session(media))return NULL;}
  else if([track[@"codec"] isEqual:@"V_VP9"]){media.width=[track[@"width"] unsignedIntValue];media.height=[track[@"height"] unsignedIntValue];media.video=track[@"packets"];
   // ISO VP codec configuration: FullBox v1, profile 0,
   // 8-bit 4:2:0 limited range; unspecified color metadata. Validate profile
   // directly from the first keyframe rather than mis-decoding other profiles.
   NSDictionary *first=media.video.firstObject;NSUInteger off=[first[@"offset"] unsignedIntegerValue];if(!first||off>=length||![first[@"key"] boolValue])return NULL;uint8_t header=bytes[off];if((header>>6)!=2||((header>>4)&1)||((header>>5)&1))return NULL;
   uint64_t pixels=(uint64_t)media.width*media.height;uint8_t level=pixels<=36864?10:pixels<=73728?11:pixels<=122880?20:pixels<=245760?21:pixels<=552960?30:pixels<=983040?31:pixels<=2228224?40:pixels<=8912896?50:60;
   uint8_t configuration[]={1,0,0,0,0,level,0x82,2,2,2,0,0};NSData *record=[NSData dataWithBytes:configuration length:sizeof(configuration)];
   NSDictionary *extensions=@{(__bridge NSString *)kCMFormatDescriptionExtension_SampleDescriptionExtensionAtoms:@{@"vpcC":record}};
   media->nativeStatus=CMVideoFormatDescriptionCreate(kCFAllocatorDefault,kCMVideoCodecType_VP9,media.width,media.height,(__bridge CFDictionaryRef)extensions,&media->format);
   if(!OrynVP9Session(media)){if(diagnostic)*diagnostic=OrynJSON(@{@"stage":@"vp9-init",@"nativeStatus":@(media->nativeStatus),@"softwareStatus":@(media->softwareStatus)});return NULL;}
  }
 }
 if(!media.video&&!media.pcm)return NULL;return (__bridge_retained void *)media;
}}
void *oryn_webm_open(const uint8_t *bytes,size_t length,const uint8_t *manifest,size_t manifestLength){return OrynWebMOpen(bytes,length,manifest,manifestLength,0,NULL);}
void *oryn_webm_open_diagnostic(const uint8_t *bytes,size_t length,const uint8_t *manifest,size_t manifestLength,char **diagnostic){return OrynWebMOpen(bytes,length,manifest,manifestLength,0,diagnostic);}
void *oryn_webm_open_for_test(const uint8_t *bytes,size_t length,const uint8_t *manifest,size_t manifestLength,unsigned mode,char **diagnostic){if(mode<1||mode>4)return NULL;return OrynWebMOpen(bytes,length,manifest,manifestLength,mode,diagnostic);}
char *oryn_webm_metadata(void *raw){@autoreleasepool{OrynWebM *media=(__bridge OrynWebM *)raw;if(!media)return NULL;return OrynJSON(@{@"duration":@(media.duration),@"width":@(media.width),@"height":@(media.height),@"audio":(media.channels!=0?@YES:@NO),@"sampleRate":@(media.channels?48000:0),@"channels":@(media.channels),@"decoder":media.video?(media->vp8Ready?@"libvpx":@"videotoolbox"):@"audio",@"nativeStatus":@(media->nativeStatus),@"softwareStatus":@(media->softwareStatus),@"injectedFailure":(media->testMode==2?@YES:@NO),@"packetFailureInjected":(media->packetFailureInjected?@YES:@NO),@"retainedBytes":@(media.bytes.length+media.pcm.length+media.manifestBytes+(media.video?OrynVPXReservation(media):0)+(NSUInteger)media.width*media.height*12)});}}
char *oryn_webm_audio(void *raw,double start,double until){@autoreleasepool{@try{
 OrynWebM *media=(__bridge OrynWebM *)raw;
 if(!media||!isfinite(start)||start<0||!isfinite(until)||until<start||until-start>0.3)return NULL;
 if(!media.pcm||until==start)return OrynJSON(@{});
 if(media.channels<1||media.channels>8)return NULL;
 NSUInteger frameBytes=media.channels*4,frames=media.pcm.length/frameBytes;
 NSUInteger first=(NSUInteger)MIN((double)frames,floor(start*48000)),last=(NSUInteger)MIN((double)frames,floor(until*48000));
 if(last<=first)return OrynJSON(@{});
 if(last-first>1024*1024/frameBytes)return NULL;
 NSData *audio=[media.pcm subdataWithRange:NSMakeRange(first*frameBytes,(last-first)*frameBytes)];
 NSString *encoded=[audio base64EncodedStringWithOptions:0];if(!audio||!encoded)return NULL;
 return OrynJSON(@{@"audio":encoded,@"sampleRate":@48000,@"channels":@(media.channels)});
}@catch(NSException *exception){return NULL;}}}
static BOOL OrynVP9Packet(OrynWebM *media,NSDictionary *packet){
 NSUInteger offset=[packet[@"offset"] unsignedIntegerValue],length=[packet[@"length"] unsignedIntegerValue];if(!length||offset>media.bytes.length||length>media.bytes.length-offset)return NO;
 if(!media->isVP8&&!OrynVP9PacketBounds((const uint8_t *)media.bytes.bytes+offset,length,media.width,media.height)){media->softwareStatus=VPX_CODEC_UNSUP_BITSTREAM;return NO;}
 if(media->vp8Ready)return OrynSoftwarePacket(media,(const uint8_t *)media.bytes.bytes+offset,length);
 CMBlockBufferRef block=NULL;CMSampleBufferRef sample=NULL;OSStatus status=CMBlockBufferCreateWithMemoryBlock(kCFAllocatorDefault,NULL,length,kCFAllocatorDefault,NULL,0,length,0,&block);
 if(status==noErr)status=CMBlockBufferReplaceDataBytes((const uint8_t *)media.bytes.bytes+offset,block,0,length);
 CMSampleTimingInfo timing={kCMTimeInvalid,CMTimeMakeWithSeconds([packet[@"time"] doubleValue],1000000),kCMTimeInvalid};
 if(status==noErr)status=CMSampleBufferCreateReady(kCFAllocatorDefault,block,media->format,1,1,&timing,1,&length,&sample);
 if(status==noErr){CFArrayRef attachments=CMSampleBufferGetSampleAttachmentsArray(sample,true);if(attachments&&CFArrayGetCount(attachments))CFDictionarySetValue((CFMutableDictionaryRef)CFArrayGetValueAtIndex(attachments,0),kCMSampleAttachmentKey_NotSync,[packet[@"key"] boolValue]?kCFBooleanFalse:kCFBooleanTrue);status=VTDecompressionSessionDecodeFrame(media->decoder,sample,0,NULL,NULL);if(status==noErr)status=VTDecompressionSessionWaitForAsynchronousFrames(media->decoder);}
 if(sample)CFRelease(sample);if(block)CFRelease(block);if(status!=noErr)media->nativeStatus=status;else if(media->decodeError!=noErr)media->nativeStatus=media->decodeError;return status==noErr&&media->decodeError==noErr;
}
char *oryn_webm_sample(void *raw,double position,double audioStart,double audioUntil){@autoreleasepool{
 OrynWebM *media=(__bridge OrynWebM *)raw;if(!media||!isfinite(position)||position<0||!isfinite(audioStart)||audioStart<0||!isfinite(audioUntil)||audioUntil<audioStart||audioUntil-audioStart>0.3)return NULL;
 NSMutableDictionary *result=[@{@"timestamp":@(position),@"ended":(position>=media.duration?@YES:@NO)}mutableCopy];
 if(media.video){NSUInteger target=0;for(NSUInteger i=0;i<media.video.count;i++){if([media.video[i][@"time"] doubleValue]<=position)target=i;else break;}
  if(media.lastPosition<0||position<media.lastPosition||position>media.lastPosition+0.25||target+1<media.cursor){NSUInteger key=target;while(key>0&&![media.video[key][@"key"] boolValue])key--;if(![media.video[key][@"key"] boolValue]||!OrynVP9Session(media))return NULL;media.cursor=key;}
  while(media.cursor<=target){
   BOOL injected=media->testMode==4&&!media->packetFailureInjected&&![media.video[media.cursor][@"key"] boolValue];
   if(injected){media->packetFailureInjected=YES;media->nativeStatus=kVTVideoDecoderMalfunctionErr;}
   if(injected||!OrynVP9Packet(media,media.video[media.cursor])){
   if((media->vp8Ready&&!injected)||media->isVP8)return NULL;
   // Native decode failed after successful setup: replay the GOP in software,
   // never feed an interframe into a fresh decoder without its reference frames.
   NSUInteger key=target;while(key>0&&![media.video[key][@"key"] boolValue])key--;
   if(![media.video[key][@"key"] boolValue])return NULL;media->softwarePreferred=YES;
   if(!OrynSoftwareSession(media))return NULL;media.cursor=key;continue;
  }media.cursor++;}media.lastPosition=position;
  if(!media->image)return NULL;CVPixelBufferRef image=media->image;size_t width=CVPixelBufferGetWidth(image),height=CVPixelBufferGetHeight(image);if(!width||!height||width>8192||height>8192||width*height*4>OrynCaptureLimit||CVPixelBufferGetPixelFormatType(image)!=kCVPixelFormatType_32BGRA)return NULL;
  if(CVPixelBufferLockBaseAddress(image,kCVPixelBufferLock_ReadOnly)!=kCVReturnSuccess)return NULL;const uint8_t *base=CVPixelBufferGetBaseAddress(image);size_t stride=CVPixelBufferGetBytesPerRow(image);NSMutableData *rgba=[NSMutableData dataWithLength:width*height*4];uint8_t *out=rgba.mutableBytes;
  if(!base||stride<width*4){CVPixelBufferUnlockBaseAddress(image,kCVPixelBufferLock_ReadOnly);return NULL;}for(size_t y=0;y<height;y++)for(size_t x=0;x<width;x++){const uint8_t *pixel=base+y*stride+x*4;uint8_t *dest=out+(y*width+x)*4;dest[0]=pixel[2];dest[1]=pixel[1];dest[2]=pixel[0];dest[3]=255;}CVPixelBufferUnlockBaseAddress(image,kCVPixelBufferLock_ReadOnly);result[@"width"]=@(width);result[@"height"]=@(height);result[@"data"]=[rgba base64EncodedStringWithOptions:0];
 }
 if(media.pcm&&audioUntil>audioStart){NSUInteger frames=media.pcm.length/(media.channels*4);NSUInteger first=(NSUInteger)MIN((double)frames,floor(audioStart*48000)),last=(NSUInteger)MIN((double)frames,floor(audioUntil*48000));if(last>first){NSData *audio=[media.pcm subdataWithRange:NSMakeRange(first*media.channels*4,(last-first)*media.channels*4)];result[@"audio"]=[audio base64EncodedStringWithOptions:0];result[@"sampleRate"]=@48000;result[@"channels"]=@(media.channels);}}
 return OrynJSON(result);
}}
void oryn_webm_close(void *raw){@autoreleasepool{OrynWebM *media=(__bridge_transfer OrynWebM *)raw;(void)media;}}
int oryn_webm_can_play(const char *mime){@autoreleasepool{
 if(!mime)return 0;NSString *value=[[NSString stringWithUTF8String:mime] lowercaseString];NSArray *parts=[value componentsSeparatedByString:@";"];NSString *kind=[parts.firstObject stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceCharacterSet];BOOL video=[kind isEqual:@"video/webm"],audio=[kind isEqual:@"audio/webm"];if(!video&&!audio)return 0;
 BOOL vp9=video,opus=YES,vp8=NO;
 for(NSString *part in [parts subarrayWithRange:NSMakeRange(1,parts.count-1)]){NSArray *pair=[part componentsSeparatedByString:@"="];if(pair.count!=2)return 0;NSString *name=[pair[0] stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceCharacterSet];if(![name isEqual:@"codecs"])return 0;NSString *list=[pair[1] stringByTrimmingCharactersInSet:[NSCharacterSet characterSetWithCharactersInString:@" \t\""]];vp9=NO;opus=NO;vp8=NO;for(NSString *raw in [list componentsSeparatedByString:@","]){NSString *codec=[raw stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceCharacterSet];if(video&&([codec isEqual:@"vp9"]||[codec isEqual:@"vp9.0"]||[codec isEqual:@"vp09.00.10.08"]))vp9=YES;else if(video&&([codec isEqual:@"vp8"]||[codec isEqual:@"vp8.0"]))vp8=YES;else if([codec isEqual:@"opus"])opus=YES;else return 0;}}
 if(opus){AudioStreamBasicDescription input={0},output={0};input.mSampleRate=48000;input.mFormatID=kAudioFormatOpus;input.mChannelsPerFrame=2;output.mSampleRate=48000;output.mFormatID=kAudioFormatLinearPCM;output.mFormatFlags=kAudioFormatFlagIsFloat|kAudioFormatFlagIsPacked;output.mBytesPerPacket=8;output.mFramesPerPacket=1;output.mBytesPerFrame=8;output.mChannelsPerFrame=2;output.mBitsPerChannel=32;AudioConverterRef converter=NULL;if(AudioConverterNew(&input,&output,&converter)!=noErr)return 0;AudioConverterDispose(converter);}
 if(vp9){OrynWebM *probe=[OrynWebM new];probe.width=16;probe.height=16;uint8_t config[]={1,0,0,0,0,10,0x82,2,2,2,0,0};NSDictionary *extensions=@{(__bridge NSString *)kCMFormatDescriptionExtension_SampleDescriptionExtensionAtoms:@{@"vpcC":[NSData dataWithBytes:config length:sizeof(config)]}};probe->nativeStatus=CMVideoFormatDescriptionCreate(kCFAllocatorDefault,kCMVideoCodecType_VP9,16,16,(__bridge CFDictionaryRef)extensions,&probe->format);if(!OrynVP9Session(probe))return 0;}
 if(vp8){vpx_codec_ctx_t probe;vpx_codec_dec_cfg_t config={1,16,16};if(vpx_codec_dec_init(&probe,vpx_codec_vp8_dx(),&config,0)!=VPX_CODEC_OK)return 0;vpx_codec_destroy(&probe);}
 return vp9||opus||vp8;
}}
