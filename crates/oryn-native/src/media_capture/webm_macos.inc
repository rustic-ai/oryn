// Explicit VP9/Opus decoder for WebM packets demuxed by webm.rs. This adapter
// has no URL API. Container byte bounds and track selection precede admission.
#import <VideoToolbox/VideoToolbox.h>
#import <dispatch/dispatch.h>
#include <vpx/vpx_decoder.h>
#include <vpx/vp8dx.h>
#include <limits.h>
@interface OrynWebM : NSObject {
@public VTDecompressionSessionRef decoder;
@public CMVideoFormatDescriptionRef format;
@public CVPixelBufferRef image;
@public OSStatus decodeError;
@public vpx_codec_ctx_t vp8;
@public BOOL vp8Ready;
@public BOOL isVP8;
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
-(void)dealloc{if(vp8Ready)vpx_codec_destroy(&vp8);if(decoder){VTDecompressionSessionInvalidate(decoder);CFRelease(decoder);}if(format)CFRelease(format);if(image)CVPixelBufferRelease(image);}
@end
static void OrynVP9Output(void *ref,void *source,OSStatus status,VTDecodeInfoFlags flags,CVImageBufferRef buffer,CMTime pts,CMTime duration){
 OrynWebM *media=(__bridge OrynWebM *)ref;if(status){media->decodeError=status;return;}if(buffer){if(media->image)CVPixelBufferRelease(media->image);media->image=CVPixelBufferRetain(buffer);}
}
static BOOL OrynVP9Session(OrynWebM *media){
 if(media->isVP8){if(media->vp8Ready){vpx_codec_destroy(&media->vp8);media->vp8Ready=NO;}if(media->image){CVPixelBufferRelease(media->image);media->image=NULL;}vpx_codec_dec_cfg_t cfg={1,media.width,media.height};media->vp8Ready=vpx_codec_dec_init(&media->vp8,vpx_codec_vp8_dx(),&cfg,0)==VPX_CODEC_OK;return media->vp8Ready;}

 // VP9 can be a supplemental decoder: opt in in this process before both
 // capability probes and actual playback create a VideoToolbox session.
 static dispatch_once_t vp9Registration;
 dispatch_once(&vp9Registration, ^{
  if (@available(macOS 11.0, *)) {
   VTRegisterSupplementalVideoDecoderIfAvailable(kCMVideoCodecType_VP9);
  }
 });

 if(media->decoder){VTDecompressionSessionInvalidate(media->decoder);CFRelease(media->decoder);media->decoder=NULL;}
 if(media->image){CVPixelBufferRelease(media->image);media->image=NULL;}media->decodeError=0;
 VTDecompressionOutputCallbackRecord callback={OrynVP9Output,(__bridge void *)media};
 NSDictionary *attributes=@{(__bridge NSString *)kCVPixelBufferPixelFormatTypeKey:@(kCVPixelFormatType_32BGRA),(__bridge NSString *)kCVPixelBufferWidthKey:@(media.width),(__bridge NSString *)kCVPixelBufferHeightKey:@(media.height)};
 return VTDecompressionSessionCreate(kCFAllocatorDefault,media->format,NULL,(__bridge CFDictionaryRef)attributes,&callback,&media->decoder)==noErr;
}
typedef struct {const uint8_t *bytes;UInt32 length;BOOL supplied;AudioStreamPacketDescription description;} OrynOpusInput;
static UInt32 OrynOpusFrames(const uint8_t *bytes,UInt32 length){if(!length)return 0;uint8_t toc=bytes[0],config=toc>>3;UInt32 frames=config>=16?(120u<<(config&3)):config>=12?(480u<<(config&1)):(UInt32[]){480,960,1920,2880}[config&3];UInt32 count=(toc&3)==0?1:(toc&3)==3?(length>1?bytes[1]&63:0):2;return count&&frames*count<=5760?frames*count:0;}
static OSStatus OrynOpusRead(AudioConverterRef converter,UInt32 *packets,AudioBufferList *data,AudioStreamPacketDescription **descriptions,void *raw){
 OrynOpusInput *input=raw;if(input->supplied){*packets=0;return noErr;}input->supplied=YES;*packets=1;data->mNumberBuffers=1;data->mBuffers[0].mNumberChannels=0;data->mBuffers[0].mData=(void *)input->bytes;data->mBuffers[0].mDataByteSize=input->length;input->description=(AudioStreamPacketDescription){0,OrynOpusFrames(input->bytes,input->length),input->length};if(descriptions)*descriptions=&input->description;return noErr;
}
static NSData *OrynDecodeOpus(NSData *bytes,NSDictionary *track,double duration){
 UInt32 channels=[track[@"channels"] unsignedIntValue];if(!channels||channels>2||!isfinite(duration)||duration<=0||duration*48000*channels*4>64*1024*1024)return nil;
 AudioStreamBasicDescription input={0};input.mSampleRate=48000;input.mFormatID=kAudioFormatOpus;input.mChannelsPerFrame=channels;
 AudioStreamBasicDescription output={0};output.mSampleRate=48000;output.mFormatID=kAudioFormatLinearPCM;output.mFormatFlags=kAudioFormatFlagIsFloat|kAudioFormatFlagIsPacked;output.mBytesPerPacket=channels*4;output.mFramesPerPacket=1;output.mBytesPerFrame=channels*4;output.mChannelsPerFrame=channels;output.mBitsPerChannel=32;
 AudioConverterRef converter=NULL;if(AudioConverterNew(&input,&output,&converter)!=noErr)return nil;
 // WebM mapping family zero is mono/stereo. The Opus pre-skip and output gain
 // are applied below in timestamp space, rather than accidentally twice through
 // a container-specific magic cookie. Multistream mapping is rejected by demux.
 NSArray *private=track[@"private"];int16_t gainBits=(int16_t)([private[16] unsignedIntValue]|([private[17] unsignedIntValue]<<8));float gain=powf(10.f,(float)gainBits/(256.f*20.f));
 NSMutableData *pcm=[NSMutableData dataWithLength:(NSUInteger)ceil(duration*48000)*channels*4];BOOL valid=YES;NSUInteger decodedFrames=0;
 for(NSDictionary *packet in track[@"packets"]){NSUInteger offset=[packet[@"offset"] unsignedIntegerValue],length=[packet[@"length"] unsignedIntegerValue];if(!length||offset>bytes.length||length>bytes.length-offset||length>UINT32_MAX){valid=NO;break;}
  OrynOpusInput supplied={(const uint8_t *)bytes.bytes+offset,(UInt32)length,NO,{0}};
  // Opus packets encode at most 120 ms. One compressed packet can produce
  // several output chunks; retain decoder state across packets for prediction.
  float decoded[5760*2];AudioBufferList buffers={1,{{channels,sizeof(decoded),decoded}}};UInt32 frames=5760;
  OSStatus status=AudioConverterFillComplexBuffer(converter,OrynOpusRead,&supplied,&frames,&buffers,NULL);if(status!=noErr||frames>5760){valid=NO;break;}decodedFrames+=frames;
  double timestamp=[packet[@"time"] doubleValue],discard=[packet[@"discard"] doubleValue];int64_t base=llround(timestamp*48000);NSUInteger first=discard<0?(NSUInteger)ceil(-discard*48000):0,last=frames;NSUInteger tail=discard>0?(NSUInteger)ceil(discard*48000):0;if(tail>last){valid=NO;break;}last-=tail;
  float *target=pcm.mutableBytes;NSUInteger total=pcm.length/(channels*4);
  for(NSUInteger frame=first;frame<last;frame++){int64_t at=base+(int64_t)frame;if(at<0)continue;if((uint64_t)at>=total)break;for(UInt32 c=0;c<channels;c++)target[at*channels+c]=decoded[frame*channels+c]*gain;}
 }
 AudioConverterDispose(converter);return valid&&decodedFrames?pcm:nil;
}
void *oryn_webm_open(const uint8_t *bytes,size_t length,const uint8_t *manifest,size_t manifestLength){@autoreleasepool{
 if(!bytes||!length||length>16*1024*1024||!manifest||manifestLength>32*1024*1024)return NULL;
 NSDictionary *description=[NSJSONSerialization JSONObjectWithData:[NSData dataWithBytes:manifest length:manifestLength] options:0 error:nil];if(![description isKindOfClass:NSDictionary.class])return NULL;
 OrynWebM *media=[OrynWebM new];media.bytes=[NSData dataWithBytes:bytes length:length];media.manifestBytes=manifestLength*2;media.duration=[description[@"duration"] doubleValue];media.lastPosition=-1;
 for(NSDictionary *track in description[@"tracks"]){if([track[@"codec"] isEqual:@"A_OPUS"]){media.channels=[track[@"channels"] unsignedIntValue];media.pcm=OrynDecodeOpus(media.bytes,track,media.duration);if(!media.pcm)return NULL;}
  else if([track[@"codec"] isEqual:@"V_VP8"]){media.width=[track[@"width"] unsignedIntValue];media.height=[track[@"height"] unsignedIntValue];media.video=track[@"packets"];media->isVP8=YES;if(!media.width||!media.height||(uint64_t)media.width*media.height*4>OrynCaptureLimit||!OrynVP9Session(media))return NULL;}
  else if([track[@"codec"] isEqual:@"V_VP9"]){media.width=[track[@"width"] unsignedIntValue];media.height=[track[@"height"] unsignedIntValue];media.video=track[@"packets"];
   // ISO VP codec configuration: FullBox v1, profile 0,
   // 8-bit 4:2:0 limited range; unspecified color metadata. Validate profile
   // directly from the first keyframe rather than mis-decoding other profiles.
   NSDictionary *first=media.video.firstObject;NSUInteger off=[first[@"offset"] unsignedIntegerValue];if(!first||off>=length||![first[@"key"] boolValue])return NULL;uint8_t header=bytes[off];if((header>>6)!=2||((header>>4)&1)||((header>>5)&1))return NULL;
   uint64_t pixels=(uint64_t)media.width*media.height;uint8_t level=pixels<=36864?10:pixels<=73728?11:pixels<=122880?20:pixels<=245760?21:pixels<=552960?30:pixels<=983040?31:pixels<=2228224?40:pixels<=8912896?50:60;
   uint8_t configuration[]={1,0,0,0,0,level,0x82,2,2,2,0,0};NSData *record=[NSData dataWithBytes:configuration length:sizeof(configuration)];
   NSDictionary *extensions=@{(__bridge NSString *)kCMFormatDescriptionExtension_SampleDescriptionExtensionAtoms:@{@"vpcC":record}};
   if(CMVideoFormatDescriptionCreate(kCFAllocatorDefault,kCMVideoCodecType_VP9,media.width,media.height,(__bridge CFDictionaryRef)extensions,&media->format)!=noErr||!OrynVP9Session(media))return NULL;
  }
 }
 if(!media.video&&!media.pcm)return NULL;return (__bridge_retained void *)media;
}}
char *oryn_webm_metadata(void *raw){@autoreleasepool{OrynWebM *media=(__bridge OrynWebM *)raw;if(!media)return NULL;return OrynJSON(@{@"duration":@(media.duration),@"width":@(media.width),@"height":@(media.height),@"audio":@(media.channels!=0),@"retainedBytes":@(media.bytes.length+media.pcm.length+media.manifestBytes+(NSUInteger)media.width*media.height*(media->isVP8?24:12))});}}
static uint8_t OrynClampVP8(int value){return (uint8_t)(value<0?0:value>255?255:value);}
static BOOL OrynVP8Packet(OrynWebM *media,const uint8_t *bytes,NSUInteger length){
 if(!media->vp8Ready||length<3||length>UINT_MAX)return NO;
 // Keyframe dimensions are visible before decoder allocation. Bound and bind
 // them to the admitted track, including keyframes reached after seeking.
 if(!(bytes[0]&1)){if(length<10||bytes[3]!=0x9d||bytes[4]!=1||bytes[5]!=0x2a)return NO;uint32_t width=(bytes[6]|bytes[7]<<8)&0x3fff,height=(bytes[8]|bytes[9]<<8)&0x3fff;if(width!=media.width||height!=media.height)return NO;}
 if(vpx_codec_decode(&media->vp8,bytes,(unsigned int)length,NULL,0)!=VPX_CODEC_OK)return NO;
 vpx_codec_iter_t iterator=NULL;vpx_image_t *decoded;while((decoded=vpx_codec_get_frame(&media->vp8,&iterator))){
  if(decoded->fmt!=VPX_IMG_FMT_I420||decoded->d_w!=media.width||decoded->d_h!=media.height||decoded->stride[0]<(int)media.width||decoded->stride[1]<(int)((media.width+1)/2)||decoded->stride[2]<(int)((media.width+1)/2)||!decoded->planes[0]||!decoded->planes[1]||!decoded->planes[2])return NO;
  CVPixelBufferRef image=NULL;if(CVPixelBufferCreate(kCFAllocatorDefault,media.width,media.height,kCVPixelFormatType_32BGRA,NULL,&image)!=kCVReturnSuccess)return NO;if(CVPixelBufferLockBaseAddress(image,0)!=kCVReturnSuccess){CVPixelBufferRelease(image);return NO;}
  uint8_t *base=CVPixelBufferGetBaseAddress(image);size_t stride=CVPixelBufferGetBytesPerRow(image);if(!base||stride<media.width*4){CVPixelBufferUnlockBaseAddress(image,0);CVPixelBufferRelease(image);return NO;}
  for(uint32_t y=0;y<media.height;y++)for(uint32_t x=0;x<media.width;x++){int yy=decoded->planes[0][y*decoded->stride[0]+x]-16,uu=decoded->planes[1][(y/2)*decoded->stride[1]+x/2]-128,vv=decoded->planes[2][(y/2)*decoded->stride[2]+x/2]-128;yy=MAX(yy,0);uint8_t *pixel=base+y*stride+x*4;pixel[2]=OrynClampVP8((298*yy+409*vv+128)>>8);pixel[1]=OrynClampVP8((298*yy-100*uu-208*vv+128)>>8);pixel[0]=OrynClampVP8((298*yy+516*uu+128)>>8);pixel[3]=255;}
  CVPixelBufferUnlockBaseAddress(image,0);if(media->image)CVPixelBufferRelease(media->image);media->image=image;
 }return YES;
}
static BOOL OrynVP9Packet(OrynWebM *media,NSDictionary *packet){
 NSUInteger offset=[packet[@"offset"] unsignedIntegerValue],length=[packet[@"length"] unsignedIntegerValue];if(!length||offset>media.bytes.length||length>media.bytes.length-offset)return NO;
 if(media->isVP8)return OrynVP8Packet(media,(const uint8_t *)media.bytes.bytes+offset,length);
 CMBlockBufferRef block=NULL;CMSampleBufferRef sample=NULL;OSStatus status=CMBlockBufferCreateWithMemoryBlock(kCFAllocatorDefault,NULL,length,kCFAllocatorDefault,NULL,0,length,0,&block);
 if(status==noErr)status=CMBlockBufferReplaceDataBytes((const uint8_t *)media.bytes.bytes+offset,block,0,length);
 CMSampleTimingInfo timing={kCMTimeInvalid,CMTimeMakeWithSeconds([packet[@"time"] doubleValue],1000000),kCMTimeInvalid};
 if(status==noErr)status=CMSampleBufferCreateReady(kCFAllocatorDefault,block,media->format,1,1,&timing,1,&length,&sample);
 if(status==noErr){CFArrayRef attachments=CMSampleBufferGetSampleAttachmentsArray(sample,true);if(attachments&&CFArrayGetCount(attachments))CFDictionarySetValue((CFMutableDictionaryRef)CFArrayGetValueAtIndex(attachments,0),kCMSampleAttachmentKey_NotSync,[packet[@"key"] boolValue]?kCFBooleanFalse:kCFBooleanTrue);status=VTDecompressionSessionDecodeFrame(media->decoder,sample,0,NULL,NULL);if(status==noErr)status=VTDecompressionSessionWaitForAsynchronousFrames(media->decoder);}
 if(sample)CFRelease(sample);if(block)CFRelease(block);return status==noErr&&media->decodeError==noErr;
}
char *oryn_webm_sample(void *raw,double position,double audioStart,double audioUntil){@autoreleasepool{
 OrynWebM *media=(__bridge OrynWebM *)raw;if(!media||!isfinite(position)||position<0||!isfinite(audioStart)||audioStart<0||!isfinite(audioUntil)||audioUntil<audioStart||audioUntil-audioStart>0.3)return NULL;
 NSMutableDictionary *result=[@{@"timestamp":@(position),@"ended":@(position>=media.duration)}mutableCopy];
 if(media.video){NSUInteger target=0;for(NSUInteger i=0;i<media.video.count;i++){if([media.video[i][@"time"] doubleValue]<=position)target=i;else break;}
  if(media.lastPosition<0||position<media.lastPosition||position>media.lastPosition+0.25||target+1<media.cursor){NSUInteger key=target;while(key>0&&![media.video[key][@"key"] boolValue])key--;if(![media.video[key][@"key"] boolValue]||!OrynVP9Session(media))return NULL;media.cursor=key;}
  while(media.cursor<=target){if(!OrynVP9Packet(media,media.video[media.cursor]))return NULL;media.cursor++;}media.lastPosition=position;
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
 if(vp9){OrynWebM *probe=[OrynWebM new];probe.width=16;probe.height=16;uint8_t config[]={1,0,0,0,0,10,0x82,2,2,2,0,0};NSDictionary *extensions=@{(__bridge NSString *)kCMFormatDescriptionExtension_SampleDescriptionExtensionAtoms:@{@"vpcC":[NSData dataWithBytes:config length:sizeof(config)]}};if(CMVideoFormatDescriptionCreate(kCFAllocatorDefault,kCMVideoCodecType_VP9,16,16,(__bridge CFDictionaryRef)extensions,&probe->format)!=noErr||!OrynVP9Session(probe))return 0;}
 if(vp8){vpx_codec_ctx_t probe;vpx_codec_dec_cfg_t config={1,16,16};if(vpx_codec_dec_init(&probe,vpx_codec_vp8_dx(),&config,0)!=VPX_CODEC_OK)return 0;vpx_codec_destroy(&probe);}
 return vp9||opus||vp8;
}}
