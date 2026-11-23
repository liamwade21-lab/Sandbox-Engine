// Sandbox Engine patch b (vendor/ruffle/UPSTREAM.md): Scaleform GFx 4's TextFieldEx. No-translate,
// vertical alignment and text auto-size are native (display_object/scaleform_text.rs); inline image
// substitution is not drawn (Ruffle's text layout has no inline images).
package scaleform.gfx {
    import flash.display.BitmapData;
    import flash.text.TextField;

    public final class TextFieldEx extends InteractiveObjectEx {
        public static const VALIGN_NONE:String = "none";
        public static const VALIGN_TOP:String = "top";
        public static const VALIGN_CENTER:String = "center";
        public static const VALIGN_BOTTOM:String = "bottom";
        public static const TEXTAUTOSZ_NONE:String = "none";
        public static const TEXTAUTOSZ_SHRINK:String = "shrink";
        public static const TEXTAUTOSZ_FIT:String = "fit";

        public static function appendHtml(textField:TextField, html:String):void {
            textField.htmlText = textField.htmlText + html;
        }

        public static function setIMEEnabled(textField:TextField, enabled:Boolean):void {
        }

        public static native function setVerticalAlign(textField:TextField, align:String):void;
        public static native function getVerticalAlign(textField:TextField):String;
        public static native function setTextAutoSize(textField:TextField, mode:String):void;
        public static native function getTextAutoSize(textField:TextField):String;
        public static native function setNoTranslate(textField:TextField, value:Boolean):void;
        public static native function getNoTranslate(textField:TextField):Boolean;

        public static function setImageSubstitutions(textField:TextField, substitutions:Object):void {
        }

        public static function updateImageSubstitution(textField:TextField, id:String, image:BitmapData):void {
        }
    }
}
