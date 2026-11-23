package elitegfx {
    import flash.display.Loader;
    import flash.external.ExternalInterface;
    import flash.utils.getDefinitionByName;

    // The communication manager Elite's exe gives every movie (its 33 native methods,
    // ui_messages_panels.md 0), with the API of the movies' own debug manager
    // (code/_shared/core/managers/CommunicationManager.as). The game's messages are delivered here;
    // everything the movie calls goes to the game through ExternalInterface under the native
    // method's name (crates/elite-gfx/src/bridge.rs), and questions are answered at once.
    public class Manager {
        // Listeners by lower-case message name, oldest first.
        private var listeners:Object = {};
        // SetGenericParam / GetGenericParam: a table of the movie's own, as the debug manager keeps.
        private var genericParams:Object = {};

        // A game message: names are case-insensitive, the newest listener hears it first.
        public function HandleMessage(name:String, ...rest):void {
            var list:Array = listeners[name.toLowerCase()];
            if (list == null || list.length == 0) {
                ExternalInterface.call("Trace", "no listener for message " + name);
                return;
            }
            for (var i:int = list.length - 1; i > -1; i--) {
                list[i].apply(null, rest);
            }
        }

        public function AddListener(name:String, listener:Function):void {
            var key:String = name.toLowerCase();
            var list:Array = listeners[key];
            if (list == null) {
                list = [];
                listeners[key] = list;
            }
            list.push(listener);
        }

        public function RemoveListener(name:String, listener:Function):void {
            var list:Array = listeners[name.toLowerCase()];
            if (list == null) {
                return;
            }
            for (var i:int = list.length - 1; i > -1; i--) {
                if (list[i] == listener) {
                    list.splice(i, 1);
                    return;
                }
            }
        }

        public function Trace(text:String):void {
            ExternalInterface.call("Trace", text);
        }

        public function SetReadyForInvokes():void {
            ExternalInterface.call("SetReadyForInvokes");
        }

        public function UIOpened():void {
            ExternalInterface.call("UIOpened");
        }

        public function UIClosed():void {
            ExternalInterface.call("UIClosed");
        }

        public function SendEventToGame(name:String, args:Array = null):void {
            ExternalInterface.call("SendEventToGame", name, args);
        }

        public function DoAction(name:String, args:Array = null):void {
            ExternalInterface.call("DoAction", name, args);
        }

        public function PlaySound(name:String, source:String = null):void {
            ExternalInterface.call("PlaySound", name, source);
        }

        public function PlayNarration(name:String, a:Boolean, b:Boolean):void {
            ExternalInterface.call("PlayNarration", name, a, b);
        }

        public function Translate(text:String):String {
            return ExternalInterface.call("Translate", text);
        }

        public function MakeUntranslatedString(text:String):String {
            return ExternalInterface.call("MakeUntranslatedString", text);
        }

        public function FindInTranslationString(text:String, find:String, from:Number):Number {
            return ExternalInterface.call("FindInTranslationString", text, find, from);
        }

        public function GetString(key:String):String {
            return ExternalInterface.call("GetString", key);
        }

        public function GetMovieName():String {
            return ExternalInterface.call("GetMovieName");
        }

        public function LocaliseNumber(value:Number, format:String, extra:String = null):String {
            return ExternalInterface.call("LocaliseNumber", value, format, extra);
        }

        public function GetDatabaseItemData(id:uint):Object {
            return ExternalInterface.call("GetDatabaseItemData", id);
        }

        public function GetData(name:String, id:uint):Object {
            return ExternalInterface.call("GetData", name, id);
        }

        public function Assert(ok:Boolean, message:String):void {
            if (!ok) {
                Break(message == null ? "Assertion failed" : message);
            }
        }

        public function Break(message:String):void {
            ExternalInterface.call("Break", message);
        }

        public function CloseMovie():void {
            getDefinitionByName("core.applications.UIApplication").instance.close();
        }

        public function CapturingButtons(on:Boolean):void {
            ExternalInterface.call("CapturingButtons", on);
        }

        public function CapturingMouse(on:Boolean):void {
            ExternalInterface.call("CapturingMouse", on);
        }

        public function OnInputEvent(name:String, state:String):void {
            ExternalInterface.call("OnInputEvent", name, state);
        }

        // The game ships no images for the movies (A10): nothing loads, and the movie's failure
        // callback (ImageLoader passes Image.loadFailureHandler) runs, so it shows its empty state.
        public function LoadImage(name:String, loader:Loader, failed:Function = null):void {
            if (failed != null) {
                failed();
            }
        }

        public function LoadMovie(name:String, loader:Loader):void {
        }

        public function Unload(loader:Loader):void {
            loader.unload();
        }

        public function DoesIconDataResourceExist(name:String):Boolean {
            return ExternalInterface.call("DoesIconDataResourceExist", name);
        }

        public function DoesTextSymbolExist(key:String):Boolean {
            return ExternalInterface.call("DoesTextSymbolExist", key);
        }

        public function OptionsFlag(name:String):Boolean {
            return ExternalInterface.call("OptionsFlag", name);
        }

        public function SetGenericParam(name:String, value:Number):void {
            genericParams[name] = value;
        }

        public function GetGenericParam(name:String):Number {
            return genericParams[name];
        }

        public function GetMappedFontName(name:String):String {
            return ExternalInterface.call("GetMappedFontName", name);
        }
    }
}
