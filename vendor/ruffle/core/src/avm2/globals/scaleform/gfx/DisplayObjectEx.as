// Sandbox Engine patch b (vendor/ruffle/UPSTREAM.md): Scaleform GFx 4's DisplayObjectEx. Batching
// and renderer strings are hints to Scaleform's own renderer; this player has nothing to give them.
package scaleform.gfx {
    import flash.display.DisplayObject;

    public class DisplayObjectEx {
        public static function disableBatching(object:DisplayObject, value:Boolean):void {
        }

        public static function isBatchingDisabled(object:DisplayObject):Boolean {
            return false;
        }

        public static function setRendererString(object:DisplayObject, value:String):void {
        }

        public static function getRendererString(object:DisplayObject):String {
            return null;
        }

        public static function setRendererFloat(object:DisplayObject, value:Number):void {
        }

        public static function getRendererFloat(object:DisplayObject):Number {
            return NaN;
        }
    }
}
