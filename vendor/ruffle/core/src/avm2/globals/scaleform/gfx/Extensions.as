// Sandbox Engine patch b (vendor/ruffle/UPSTREAM.md): Scaleform GFx 4's AS3 extension classes as
// built-ins, so they shadow the empty stand-ins Scaleform movies carry (a definition in the system
// domain wins). Written from Scaleform's documented behaviour; no Scaleform code.
package scaleform.gfx {
    import flash.display.DisplayObject;
    import flash.geom.Rectangle;
    import flash.text.TextField;

    public final class Extensions {
        public static const EDGEAA_INHERIT:uint = 0;
        public static const EDGEAA_ON:uint = 1;
        public static const EDGEAA_OFF:uint = 2;
        public static const EDGEAA_DISABLE:uint = 3;

        // A game, not Scaleform's own viewer.
        public static var isGFxPlayer:Boolean = false;
        public static var CLIK_addedToStageCallback:Function;
        public static var gfxProcessSound:Function;

        private static var _enabled:Boolean = false;

        // The movie runs in Scaleform: the movies' Scaleform branches are taken.
        public static function get isScaleform():Boolean {
            return true;
        }

        public static function get enabled():Boolean {
            return _enabled;
        }

        public static function set enabled(value:Boolean):void {
            _enabled = value;
        }

        // Invisible movie clips' timelines do not advance (native: the player skips them).
        public static native function get noInvisibleAdvance():Boolean;
        public static native function set noInvisibleAdvance(value:Boolean):void;

        // Mouse picking, cursors and edge anti-aliasing are not part of this player: neutral answers.
        public static function getTopMostEntity(x:Number, y:Number, testAll:Boolean = true):DisplayObject {
            return null;
        }

        public static function getMouseTopMostEntity(testAll:Boolean = true, mouseIndex:uint = 0):DisplayObject {
            return null;
        }

        public static function setMouseCursorType(cursor:String, mouseIndex:uint = 0):void {
        }

        public static function getMouseCursorType(mouseIndex:uint = 0):String {
            return "";
        }

        public static function get numControllers():uint {
            return 1;
        }

        public static function get visibleRect():Rectangle {
            return new Rectangle(0, 0, 0, 0);
        }

        public static function getEdgeAAMode(object:DisplayObject):uint {
            return EDGEAA_INHERIT;
        }

        public static function setEdgeAAMode(object:DisplayObject, mode:uint):void {
        }

        public static function setIMEEnabled(textField:TextField, enabled:Boolean):void {
        }
    }
}
