/* SPDX-License-Identifier: MIT. Real text-input-v3 and field-geometry test client. */
#define _GNU_SOURCE
#include <assert.h>
#include <cairo.h>
#include <fcntl.h>
#include <poll.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <time.h>
#include <unistd.h>
#include <wayland-client.h>
#include "xdg-shell-client.h"
#include "text-input-client.h"
#include "text-field-client.h"

static struct wl_display *display;
static struct wl_compositor *compositor;
static struct wl_shm *shm;
static struct wl_seat *seat;
static struct xdg_wm_base *wm;
static struct zwp_text_input_manager_v3 *input_manager;
static struct aqua_text_field_manager_v1 *field_manager;
static struct wl_surface *surface;
static struct xdg_surface *xdg_surface;
static struct zwp_text_input_v3 *input;
static struct aqua_text_field_v1 *field;
static char text[4097] = "hel";
static char pending[4097];
static uint32_t delete_before, delete_after;
static int configured, enabled, closed, password, cancel_after_first, invalid_offset, events;
static int width=760, height=400;
static double last_event;
static double now_ms(void) { struct timespec t; clock_gettime(CLOCK_MONOTONIC, &t); return t.tv_sec*1000.0+t.tv_nsec/1e6; }
static void buffer_release(void *data, struct wl_buffer *buffer) { munmap(data, width*height*4); wl_buffer_destroy(buffer); }
static const struct wl_buffer_listener buffer_listener = {.release=buffer_release};
static void draw(void) {
    if (!configured) return;
    int fd=memfd_create("aqua-input-test", MFD_CLOEXEC); assert(fd>=0);
    int bytes=width*height*4; assert(ftruncate(fd,bytes)==0);
    void *pixels=mmap(NULL,bytes,PROT_READ|PROT_WRITE,MAP_SHARED,fd,0); assert(pixels!=MAP_FAILED);
    struct wl_shm_pool *pool=wl_shm_create_pool(shm,fd,bytes);
    struct wl_buffer *buffer=wl_shm_pool_create_buffer(pool,0,width,height,width*4,WL_SHM_FORMAT_ARGB8888);
    wl_shm_pool_destroy(pool);close(fd);wl_buffer_add_listener(buffer,&buffer_listener,pixels);
    cairo_surface_t *image=cairo_image_surface_create_for_data(pixels,CAIRO_FORMAT_ARGB32,width,height,width*4);
    cairo_t *c=cairo_create(image);
    cairo_set_source_rgb(c,0.96,0.965,0.98);cairo_paint(c);
    cairo_select_font_face(c,"Sans",CAIRO_FONT_SLANT_NORMAL,CAIRO_FONT_WEIGHT_BOLD);
    cairo_set_source_rgb(c,0.1,0.12,0.18);cairo_set_font_size(c,25);cairo_move_to(c,48,63);cairo_show_text(c,"Text that keeps up with you.");
    cairo_select_font_face(c,"Sans",CAIRO_FONT_SLANT_NORMAL,CAIRO_FONT_WEIGHT_NORMAL);
    cairo_set_font_size(c,14);cairo_set_source_rgb(c,0.4,0.42,0.47);cairo_move_to(c,48,94);cairo_show_text(c,"Native suggestions. One grapheme at a time. No clipboard.");
    cairo_set_source_rgb(c,1,1,1);cairo_rectangle(c,48,130,640,56);cairo_fill(c);
    cairo_set_source_rgb(c,0.27,0.5,0.95);cairo_set_line_width(c,2);cairo_rectangle(c,48,130,640,56);cairo_stroke(c);
    cairo_set_source_rgb(c,0.1,0.12,0.18);cairo_set_font_size(c,21);cairo_move_to(c,65,166);
    cairo_show_text(c,password ? "••••••••" : text);
    double caret_x=65+strlen(text)*11; if(password) caret_x=166;
    cairo_set_source_rgb(c,0.2,0.45,0.95);cairo_rectangle(c,caret_x,145,2,26);cairo_fill(c);
    cairo_set_source_rgb(c,0.45,0.46,0.51);cairo_set_font_size(c,12);cairo_move_to(c,48,365);
    cairo_show_text(c,password ? "Passwords stay in your system store. Select an entry to fill." : "Arrow keys to choose  •  Tab to insert  •  Escape to dismiss");
    cairo_destroy(c);cairo_surface_destroy(image);
    wl_surface_attach(surface,buffer,0,0);wl_surface_damage(surface,0,0,width,height);wl_surface_commit(surface);
}
static void publish(void) {
    if (!enabled) return;
    zwp_text_input_v3_set_content_type(input,password ? ZWP_TEXT_INPUT_V3_CONTENT_HINT_HIDDEN_TEXT | ZWP_TEXT_INPUT_V3_CONTENT_HINT_SENSITIVE_DATA : ZWP_TEXT_INPUT_V3_CONTENT_HINT_COMPLETION,
        password ? ZWP_TEXT_INPUT_V3_CONTENT_PURPOSE_PASSWORD : ZWP_TEXT_INPUT_V3_CONTENT_PURPOSE_NORMAL);
    zwp_text_input_v3_set_surrounding_text(input,text, invalid_offset ? 99999 : (int32_t)strlen(text),(int32_t)strlen(text));
    int x=password ? 166 : 65+(int)strlen(text)*11;
    zwp_text_input_v3_set_cursor_rectangle(input,x,145,2,26);
    zwp_text_input_v3_commit(input);
    if(field) { aqua_text_field_v1_set_geometry(field,wl_fixed_from_int(48),wl_fixed_from_int(130),wl_fixed_from_int(640),wl_fixed_from_int(56),wl_fixed_from_int(12));wl_surface_commit(surface); }
}
static void input_enter(void *data,struct zwp_text_input_v3 *ti,struct wl_surface *s) {
    (void)data;(void)s;if(enabled)return;enabled=1;
    zwp_text_input_v3_enable(ti);publish();fprintf(stderr,"INPUT_ENABLED mode=%s\n",password?"password":"normal");
}
static void input_leave(void *data,struct zwp_text_input_v3 *ti,struct wl_surface *s) { (void)data;(void)s;enabled=0;zwp_text_input_v3_disable(ti);zwp_text_input_v3_commit(ti); }
static void preedit(void*d,struct zwp_text_input_v3*t,const char*s,int32_t a,int32_t b){(void)d;(void)t;(void)s;(void)a;(void)b;}
static void commit_string(void*d,struct zwp_text_input_v3*t,const char*s) {
    (void)d;(void)t;snprintf(pending,sizeof pending,"%s",s?s:"");
    if(s && *s) {
        double stamp=now_ms(), gap=last_event?stamp-last_event:0;last_event=stamp;events++;
        fprintf(stderr,"INSERT event=%d bytes=%zu gap_ms=%.3f\n",events,strlen(s),gap);
        assert(strlen(s)<=4); /* Test data consists of individual ASCII/Cyrillic graphemes. */
        if(events>1) assert(gap>=3.0);
    }
}
static void delete_text(void*d,struct zwp_text_input_v3*t,uint32_t a,uint32_t b){(void)d;(void)t;delete_before=a;delete_after=b;}
static void done(void*d,struct zwp_text_input_v3*t,uint32_t serial){
    (void)d;(void)t;(void)serial;assert(delete_after==0);
    size_t n=strlen(text);assert(delete_before<=n);text[n-delete_before]=0;
    assert(strlen(text)+strlen(pending)<sizeof text);strcat(text,pending);
    delete_before=delete_after=0;pending[0]=0;
    if(cancel_after_first && events==1){
        zwp_text_input_v3_disable(input);zwp_text_input_v3_commit(input);enabled=0;
        if(field)aqua_text_field_v1_clear(field);
        fprintf(stderr,"INPUT_DISABLED_AFTER_FIRST\n");
    }else publish();
    draw();if(!password)fprintf(stderr,"TEXT=%s\n",text);
}
static const struct zwp_text_input_v3_listener input_listener={.enter=input_enter,.leave=input_leave,.preedit_string=preedit,.commit_string=commit_string,.delete_surrounding_text=delete_text,.done=done};
static void ping(void*d,struct xdg_wm_base*b,uint32_t s){(void)d;xdg_wm_base_pong(b,s);}
static const struct xdg_wm_base_listener wm_listener={.ping=ping};
static void configure(void*d,struct xdg_surface*s,uint32_t serial){(void)d;xdg_surface_ack_configure(s,serial);configured=1;draw();}
static const struct xdg_surface_listener surface_listener={.configure=configure};
static void top_configure(void*d,struct xdg_toplevel*t,int32_t w,int32_t h,struct wl_array*s){(void)d;(void)t;(void)w;(void)h;(void)s;}
static void top_close(void*d,struct xdg_toplevel*t){(void)d;(void)t;closed=1;}
static const struct xdg_toplevel_listener top_listener={.configure=top_configure,.close=top_close};
static void global(void*d,struct wl_registry*r,uint32_t name,const char*iface,uint32_t version){
    (void)d;(void)version;
#define BIND(NAME,VAR,TYPE) if(!strcmp(iface,NAME))VAR=wl_registry_bind(r,name,&TYPE,1)
    BIND("wl_compositor",compositor,wl_compositor_interface);
    else BIND("wl_shm",shm,wl_shm_interface);
    else BIND("wl_seat",seat,wl_seat_interface);
    else BIND("xdg_wm_base",wm,xdg_wm_base_interface);
    else BIND("zwp_text_input_manager_v3",input_manager,zwp_text_input_manager_v3_interface);
    else BIND("aqua_text_field_manager_v1",field_manager,aqua_text_field_manager_v1_interface);
#undef BIND
}
static void removed(void*d,struct wl_registry*r,uint32_t n){(void)d;(void)r;(void)n;}
static const struct wl_registry_listener registry_listener={.global=global,.global_remove=removed};
int main(int argc,char**argv){
    const char*mode=argc>1?argv[1]:"plain";
    password=!strcmp(mode,"password");cancel_after_first=!strcmp(mode,"cancel");invalid_offset=!strcmp(mode,"invalid");
    if(password)snprintf(text,sizeof text,"test-only");
    if(!strcmp(mode,"unicode"))snprintf(text,sizeof text,"при");
    if(!strcmp(mode,"t9"))snprintf(text,sizeof text,"43556");
    display=wl_display_connect(NULL);assert(display);
    struct wl_registry*registry=wl_display_get_registry(display);wl_registry_add_listener(registry,&registry_listener,NULL);assert(wl_display_roundtrip(display)>=0);
    assert(compositor&&shm&&wm&&seat&&input_manager&&field_manager);xdg_wm_base_add_listener(wm,&wm_listener,NULL);
    surface=wl_compositor_create_surface(compositor);xdg_surface=xdg_wm_base_get_xdg_surface(wm,surface);xdg_surface_add_listener(xdg_surface,&surface_listener,NULL);
    struct xdg_toplevel*top=xdg_surface_get_toplevel(xdg_surface);xdg_toplevel_add_listener(top,&top_listener,NULL);
    xdg_toplevel_set_title(top,"Aqua — native input test");xdg_toplevel_set_app_id(top,"org.aqua.InputTest");
    input=zwp_text_input_manager_v3_get_text_input(input_manager,seat);zwp_text_input_v3_add_listener(input,&input_listener,NULL);
    field=aqua_text_field_manager_v1_get_field(field_manager,surface);
    wl_surface_commit(surface);double end=now_ms()+16000;
    while(!closed&&now_ms()<end){
        while(wl_display_prepare_read(display)!=0)if(wl_display_dispatch_pending(display)<0)goto finished;
        wl_display_flush(display);struct pollfd p={.fd=wl_display_get_fd(display),.events=POLLIN};
        int ready=poll(&p,1,100);
        if(ready>0){if(wl_display_read_events(display)<0)break;if(wl_display_dispatch_pending(display)<0)break;}else wl_display_cancel_read(display);
    }
finished:
    fprintf(stderr,"SUMMARY events=%d mode=%s\n",events,mode);wl_display_disconnect(display);return 0;
}
