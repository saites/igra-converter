import { ref, computed } from 'vue'

export default {
  props: {
    data: Object,
  },
  setup(props) {

    return {
    }
  },
  template: `
      <table class="table-auto">
        <thead>
          <tr>
            <th>Property</th>
            <th>Value</th>
          </tr>
        </thead>
        <tbody>
          <tr>
            <td>IGRA Number</td>
            <td>{{data.igra_number}}</td>
          </tr>

          <tr>
            <td>Association</td>
            <td>{{data.association}}</td>
          </tr>

          <tr>
            <td>Legal Name</td>
            <td>{{data.first_name}} {{data.last_name}}</td>
          </tr>

          <tr>
            <td>Performance Name</td>
            <td>{{data.first_name}} {{data.last_name}}</td>
          </tr>

          <tr>
            <td>Birthdate (YYYYMMDD)</td>
            <td>{{data.birthdate}}</td>
          </tr>

          <tr>
            <td>Competes With</td>
            <td>{{data.sex == "M" ? "Cowboys" : "Cowgirls"}}</td>
          </tr>

          <tr>
            <td>Address</td>
            <td>{{data.address}}<br>{{data.city}}, {{data.state}} {{data.zip}}</td>
          </tr>

          <tr>
            <td>Email Address</td>
            <td>{{data.email}}</td>
          </tr>

          <tr>
            <td>Cell Phone</td>
            <td>{{data.cell_phone}}</td>
          </tr>

        </tbody>
      </table>
  `
}