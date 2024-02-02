package <#= package_name #>;

import android.app.Activity;
import android.os.Bundle;

import <#= package_name #>.R;

public class <#= class_name #> extends Activity {

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        <# match xml_layout {Some(layout) => { #>
        setContentView(R.layout.<#= layout #>);
        <# }, None =>{}} #>

    }

}
